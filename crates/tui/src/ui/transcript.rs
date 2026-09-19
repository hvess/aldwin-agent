//! The conversation log: one `LogEntry` at a time turned into rows, the
//! welcome hero that stands in for an empty log, and the panel that scrolls
//! them.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::diff;
use super::grid::{elide, justified_line, with_label_column, Ctx, MARGIN_X};
use super::markdown::{self, Segment};
use super::row::{band_row, Row};
use super::wrap::wrap_line;
use mjolnir_permissions::PromptPayload;

use crate::app::{App, PermState};
use crate::highlight;
use crate::log::{LogEntry, ToolActivityStatus};

/// Builds every line the log panel's *inner* area can show, at `ctx.width`
/// × `height` (the panel's inner rect — see `super::draw` on why this must
/// be the inner, not outer, rect). An empty log shows the welcome hero
/// instead of any entries — the two are mutually exclusive, so there's no
/// "separate the banner from the first real entry" case.
///
/// Every line this returns is **already one screen row**: no caller wraps
/// afterwards, so `lines.len()` *is* the row count and row `n` of the block
/// is row `n` of it on screen. See [`Transcript`] for why that equivalence
/// is the whole point.
///
/// `first` says this is the first entry in the log that rendered anything at
/// all, which is what decides whether the block opens with a separator. It
/// is *not* `index == 0`: an entry can render to nothing (a routine
/// `TurnEnded` is folded into the status line's activity indicator instead
/// of getting its own row), and a silent entry must not leave a blank one
/// behind.
fn block_rows(entry: &LogEntry, first: bool, opens: bool, ctx: Ctx) -> Vec<Line<'static>> {
    let rendered = render_entry(entry, opens, ctx);
    if rendered.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<Line<'static>> = Vec::new();
    if !first {
        // A fresh `UserMessage`/`AssistantText` starts a new conversational
        // turn and gets a turn break between it and whatever came before.
        // That break is a *band*, not a rule: the design system's Turn 13
        // rebuild replaced every freestanding rule with "one full-width row
        // of the composer's tone", and says of the terminal case that "in a
        // terminal that is a single `Style::bg` on a one-row rect, so
        // nothing here needs approximating". So this is a row of `break_`
        // with no glyph in it at all — the tonal step off the transcript
        // ground is the whole separator.
        //
        // Retry/error/notice entries continue the current turn rather than
        // starting a new one, so they only get the plain blank row a turn's
        // own internal groups get — and so does an agent entry that follows
        // another agent entry, a tool group answering the prose above it
        // being one turn rather than two.
        if opens {
            lines.push(Line::default());
            lines.push(band_row(ctx.pal.break_, ctx));
            lines.push(Line::default());
        } else {
            lines.push(Line::default());
        }
    }
    lines.extend(rendered);
    // No spinner row is appended here — per explicit developer feedback, an
    // active turn used to get an animated "thinking…"/"working…" row both
    // here (trailing the log) *and* in the status line right above the
    // input, which read as a plain duplicate of the same information. The
    // status line is now the one place live turn activity shows.
    lines
}

/// Whose turn an entry belongs to. Not every entry has one: a retry, an
/// error and a notice all continue whatever turn is open rather than
/// starting one, which is why this is an `Option` at every call site.
///
/// The agent's arm is the load-bearing half. A tool call used to render
/// above the turn break rather than below it, so it was grouped into the
/// *developer's* turn and its label column was empty — an unattributed row
/// at cells 3–10. `4a` makes the tool group part of the agent's turn
/// ("Agent prose is neutral-300. One blank row between prose and a tool
/// group"), and in Mjolnir the call comes before the reply it produces, so
/// it is the tool group that opens that turn and therefore carries its
/// speaker label.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Speaker {
    You,
    Agent,
}

/// Which turn `entry` belongs to, or `None` if it continues whichever one
/// is open.
fn speaker(entry: &LogEntry) -> Option<Speaker> {
    match entry {
        LogEntry::UserMessage { .. } => Some(Speaker::You),
        LogEntry::AssistantText { .. }
        | LogEntry::ToolActivity { .. }
        | LogEntry::ApprovalCard { .. }
        | LogEntry::PermissionPrompt { .. } => Some(Speaker::Agent),
        LogEntry::RetryAttempt { .. } | LogEntry::TurnEnded { .. } | LogEntry::Error { .. } | LogEntry::Notice { .. } => None,
    }
}

/// The transcript's screen rows, kept **one log entry at a time**.
///
/// Two properties, and the second is the reason this is a struct rather than
/// a function.
///
/// **A row here is a row on screen.** Nothing downstream wraps, so
/// [`Transcript::len`] is exactly the number of terminal rows the
/// conversation occupies and `ScrollState::offset` indexes straight into it.
/// That equivalence used to be established the other way round — the
/// builders emitted logical lines, the log's `Paragraph` wrapped them, and
/// the count came from `Paragraph::line_count` running the same wrapper a
/// second time (see mjolnir-tui.md's 2026-08-29 notes for the two bugs that
/// got it there). Correct, but it meant three passes over the whole
/// conversation per frame. Wrapping in the builders makes the count free and
/// the render O(viewport), and closes the divergence the old discipline
/// could only ever *document*, since there is no second wrapper left to
/// disagree with the first. The obligation moves to the builders: every arm
/// of [`render_entry`] must emit rows that already fit their column, because
/// an over-wide one is now truncated rather than wrapped.
///
/// **A rebuild costs one entry, not the conversation.** Caching the whole
/// flat row list was still wrong in the one place it mattered most: a
/// streaming reply appends to the *last* log entry once per token, and each
/// append invalidated everything — so every token re-parsed every diff and
/// re-highlighted every code fence in the session through syntect. Measured
/// on a real session against a local streaming endpoint: **58% of a core at
/// four turns, 95% at eight**, rising linearly with the transcript, which is
/// what "pinned to 100 while it answers" actually was. In-process it splits
/// as 2.9 / 5.2 / 10.1 ms per token at 93 / 189 / 381 rows, against a flat
/// 0.45 ms for the draw itself.
///
/// So each entry keeps its own rows beside a copy of the entry they were
/// built from, and [`Transcript::sync`] re-renders only the entries whose
/// value actually changed. The key is the entry itself compared with `==`,
/// not a fingerprint derived from it: a fingerprint is a second statement of
/// what "changed" means and can silently disagree with the first, and
/// `String`'s own comparison already short-circuits on length, which is the
/// case that matters here.
#[derive(Default)]
pub(crate) struct Transcript {
    /// What `blocks` was built against — the two inputs `Ctx` carries into
    /// every builder, and neither changes at a rate worth being incremental
    /// about. Deliberately *not* the band's height: see [`Transcript::sync`].
    width:  u16,
    theme:  Option<crate::palette::Theme>,
    /// Parallel to `App::log`, in the same order.
    blocks: Vec<CachedBlock>,
    /// `starts[i]` is the screen row `blocks[i]` begins on; one longer than
    /// `blocks`, so the last element is the total row count.
    starts: Vec<usize>,
    /// The welcome hero, which stands in for the entries when the log is
    /// empty. Never cached across syncs — it is seven rows, and it reads
    /// `App::status`, which changes on its own schedule.
    hero:   Vec<Line<'static>>,
}

struct CachedBlock {
    /// The value `rows` was rendered from. Cloned, so the comparison next
    /// sync is against what was actually drawn rather than against a summary
    /// of it.
    entry: LogEntry,
    /// Whether it rendered as the first entry to produce anything — the
    /// other half of `block_rows`' input, and it can change without `entry`
    /// changing (an earlier entry falling silent), so it is part of the key.
    first: bool,
    /// Whether it opened a new turn, which decides both its separator and
    /// whether it carries a speaker label. Like `first` it depends on what
    /// came before rather than on the entry itself, so it is part of the
    /// key too.
    opens: bool,
    rows:  Vec<Line<'static>>,
}

impl CachedBlock {
    /// Rows this block spends separating itself from the one above: the
    /// blank / break band / blank of a new turn, the single blank row of a
    /// continuation, or nothing at all for the first block in the log.
    ///
    /// Known per block rather than recognised by looking at the rows, which
    /// is what lets [`Transcript::viewport`] tell a turn break at the top
    /// of the screen from an ordinary blank row inside a reply.
    fn lead(&self) -> usize {
        match (self.rows.is_empty() || self.first, self.opens) {
            (true, _) => 0,
            (false, true) => 3,
            (false, false) => 1,
        }
    }
}

impl Transcript {
    /// Brings the cache up to date with `app` at `width` × `height`,
    /// re-rendering only what changed. Cheap enough to call every frame, and
    /// it must be: it is the one place that knows what the transcript
    /// currently is.
    pub(crate) fn sync(&mut self, app: &App, width: u16, height: u16) {
        let theme = app.theme;
        // `width` and `theme` only. `height` is *not* an input to
        // `block_rows` — no arm of `render_entry` reads it — and keying the
        // cache on it anyway meant that anything which resized the log band
        // threw away every rendered row in the session and rebuilt it.
        //
        // That band is resized by the composer growing, which used to happen
        // only on an explicit newline and now happens whenever a draft
        // wraps: measured at 7.2x the steady-state frame cost on a 40-turn
        // transcript, rising with the session, on *ordinary typing* across a
        // wrap column. The hero *is* laid out against `height`, and takes it
        // as the parameter below — it is rebuilt on every sync regardless,
        // so it needs no cache key of its own.
        if self.width != width || self.theme != Some(theme) {
            self.blocks.clear();
            self.width = width;
            self.theme = Some(theme);
        }
        let ctx = Ctx::new(theme.palette(), width);

        if app.log.is_empty() {
            self.blocks.clear();
            self.starts.clear();
            self.hero = hero_lines(app, height, ctx);
            return;
        }
        self.hero = Vec::new();
        // A shorter log means entries were dropped (`/clear`, a history
        // rewrite); the tail of the cache describes rows that no longer
        // exist.
        self.blocks.truncate(app.log.len());

        let mut first = true;
        // Whose turn is open. An entry that renders nothing leaves it
        // alone, for the same reason it leaves `first` alone: a turn the
        // screen never showed cannot be the one a label belongs to.
        let mut open: Option<Speaker> = None;
        for (i, entry) in app.log.iter().enumerate() {
            let speaker = speaker(entry);
            let opens = speaker.is_some() && speaker != open;
            let hit = matches!(self.blocks.get(i), Some(b) if b.first == first && b.opens == opens && b.entry == *entry);
            if !hit {
                let block = CachedBlock { entry: entry.clone(), first, opens, rows: block_rows(entry, first, opens, ctx) };
                match self.blocks.get_mut(i) {
                    Some(slot) => *slot = block,
                    None => self.blocks.push(block),
                }
            }
            if !self.blocks[i].rows.is_empty() {
                first = false;
                open = speaker.or(open);
            }
        }

        self.starts.clear();
        self.starts.reserve(self.blocks.len() + 1);
        let mut acc = 0;
        self.starts.push(acc);
        for block in &self.blocks {
            acc += block.rows.len();
            self.starts.push(acc);
        }
    }

    /// Total screen rows — what `ScrollState` measures its offset against.
    pub(crate) fn len(&self) -> usize {
        if self.blocks.is_empty() {
            return self.hero.len();
        }
        self.starts.last().copied().unwrap_or(0)
    }

    /// The rows to *draw* for a viewport of `height` rows starting at
    /// `offset` — [`Transcript::slice`] with one correction, and the
    /// correction is the whole reason the method exists.
    ///
    /// A turn break belongs to the turn below it, so when the turn above
    /// has scrolled off the top the band comes down anyway and separates
    /// the frame's first content from nothing at all. Measured at 80×24,
    /// where the transcript band is 16 rows and the conversation is 17: the
    /// `you` turn's one row was cut, and the frame opened on a blank row, a
    /// band, and another blank row — three of sixteen rows spent marking a
    /// boundary between the agent's reply and the top of the screen.
    ///
    /// So a leading separator the viewport *starts inside* gives up its two
    /// blank rows, and the rows that frees go to the turn above: the band
    /// itself is redrawn directly under whatever of that turn fits. The
    /// blanks are the separator's breathing room and the band is the
    /// separator, so under pressure it is the breathing room that goes —
    /// at 80×24 that is the difference between a frame showing the question
    /// and its answer with a boundary between them, and a frame showing
    /// three blank rows and an unattributed reply.
    ///
    /// When nothing of the turn above fits, the band goes too: a boundary
    /// drawn against the top of the viewport separates the reply from
    /// nothing at all, which is the defect this started as.
    ///
    /// Nothing here changes [`Transcript::len`] or the scroll offset — what
    /// the viewport draws is not how long the conversation is. That is
    /// deliberate: feeding it back into the row count would shorten the
    /// transcript, un-scroll the turn it hid, and bring the band back on the
    /// next frame, which is a flicker rather than a layout.
    pub(crate) fn viewport(&self, offset: usize, height: usize) -> Vec<Line<'static>> {
        let end = offset + height;
        let Some((block, start)) = self.block_at(offset) else { return self.slice(offset, height) };
        let lead = block.lead();
        if offset >= start + lead {
            return self.slice(offset, height);
        }
        // The block's own content, and everything after it.
        let content = start + lead;
        let mut rows = self.slice(content, end.saturating_sub(content));
        // What the dropped separator freed, spent on the turn above and on
        // the one row of it that is a boundary rather than a gap.
        let free = height.saturating_sub(rows.len());
        let above = free.saturating_sub(1).min(start);
        if above == 0 {
            return rows;
        }
        let mut out = self.slice(start - above, above);
        // `lead == 3` is a turn break: blank, band, blank. A continuation's
        // single blank row has no band in it to keep.
        if lead == 3 {
            out.push(block.rows[1].clone());
        }
        out.append(&mut rows);
        out
    }

    /// The block `offset` falls in, and the row that block starts on.
    fn block_at(&self, offset: usize) -> Option<(&CachedBlock, usize)> {
        let i = self.starts.partition_point(|&s| s <= offset).saturating_sub(1);
        Some((self.blocks.get(i)?, self.starts.get(i).copied().unwrap_or(0)))
    }

    /// The `count` rows starting at `offset`, or fewer at the end. Owned,
    /// because the rows come from several blocks and a viewport is at most a
    /// terminal's height — copying forty `Line`s is not worth a lifetime.
    pub(crate) fn slice(&self, offset: usize, count: usize) -> Vec<Line<'static>> {
        if self.blocks.is_empty() {
            let end = self.hero.len().min(offset.saturating_add(count));
            return self.hero.get(offset..end).unwrap_or(&[]).to_vec();
        }
        // The last block that starts at or before `offset`. Blocks that
        // render to nothing share a start with their neighbour; landing on
        // one is harmless, since the walk below simply steps past it.
        let mut i = self.starts.partition_point(|&s| s <= offset).saturating_sub(1).min(self.blocks.len());
        let mut row = offset.saturating_sub(self.starts.get(i).copied().unwrap_or(0));
        // Reserve for what is actually there, not for what was asked for —
        // `count` is a caller's upper bound and may be far past the end.
        let mut out = Vec::with_capacity(count.min(self.len().saturating_sub(offset)));
        while out.len() < count && i < self.blocks.len() {
            let rows = &self.blocks[i].rows;
            while row < rows.len() && out.len() < count {
                out.push(rows[row].clone());
                row += 1;
            }
            i += 1;
            row = 0;
        }
        out
    }
}

/// Renders the log panel: `block` onto `outer`, and `visible` — the rows
/// `super::draw` already sliced out of the [`Transcript`] — onto `inner`.
///
/// **No scrollbar.** The design system lists scrollbars under "Deliberately
/// absent", beside tabs, breadcrumbs and "any control that needs a mouse",
/// and this one was drawing a `║` track down column 119 whenever the
/// transcript overflowed — the one thing in the frame that sat outside the
/// grid's right margin. The log still scrolls; what is gone is the drawn
/// indicator of it. Nothing replaced it: the transcript is bottom-anchored,
/// so the live end of the conversation is always the thing on screen, and
/// the design's answer to "where am I" is that you are at the bottom unless
/// you moved.
pub(super) fn draw_log(frame: &mut Frame, outer: Rect, inner: Rect, block: Block<'static>, visible: Vec<Line<'static>>) {
    frame.render_widget(block, outer);
    // No `Wrap`: these rows are already one screen row each, so there is
    // nothing for a wrapper to do and a `Paragraph` is just a blitter here.
    frame.render_widget(Paragraph::new(Text::from(visible)), inner);
}

/// One hand-composed row's worth of spans, wrapped to the turn body column
/// and then laid out under the label column — in that order, which is this
/// module's whole discipline (see its own doc comment) and now also the
/// thing that keeps [`rows`]'s one-line-per-screen-row invariant true for
/// the status-ish entries below. Each of them carries arbitrary text — a
/// provider's retry message, a tool error, a slash command's notice — so
/// "it's short enough" was never a property any of them actually had; they
/// simply used to be wrapped by the log's `Paragraph` afterwards, which
/// stranded the continuation row against the frame's left edge.
fn body_lines(spans: Vec<Span<'static>>, label: Option<(&str, Color)>, ctx: Ctx) -> Vec<Line<'static>> {
    with_label_column(wrap_line(Line::from(spans), ctx.body().width as usize), label)
}

/// `opens` says this entry is the first of its turn, which is what decides
/// whether it carries a speaker label: `4a` puts the speaker on the label
/// column's first row *of the turn*, so a reply that continues a turn its
/// own tool group already opened does not repeat the word.
fn render_entry(entry: &LogEntry, opens: bool, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    let label = |text: &'static str, color| opens.then_some((text, color));
    match entry {
        // `Turn.jsx`: the `you` label in `speaker-you` (accent-toned), the
        // `harness` label in `speaker-agent` (neutral) — content in `text`
        // (primary) for a `you` turn, `body` for the agent's. No filled
        // background: the design system's own components never fill a chat
        // message's background — flat coloured text on the panel ground is
        // the whole treatment. A slash command (directed at the harness,
        // not the model) skips the speaker label too, since it isn't
        // conversational content.
        LogEntry::UserMessage { text } => {
            if is_command(text) {
                return text
                    .lines()
                    .flat_map(|l| wrap_line(Line::from(Span::styled(format!("> {l}"), Style::default().fg(pal.dim))), ctx.width as usize))
                    .collect();
            }
            let style = Style::default().fg(pal.text);
            let content: Vec<Line<'static>> =
                text.lines().flat_map(|l| wrap_line(Line::from(Span::styled(l.to_string(), style)), ctx.body().width as usize)).collect();
            with_label_column(content, label("you", pal.speaker_you))
        }
        LogEntry::AssistantText { text } => with_label_column(render_assistant_text(text, ctx), label("harness", pal.speaker_agent)),
        // `ToolLine.jsx`: a status glyph, the tool name, a right-flush
        // result summary. `ToolActivityEntry` carries no separate target
        // path distinct from the tool's own name (unlike the reference's
        // `read src/gateway/mod.rs`), so the call id stands in for it,
        // parenthesized. The design system's glyph table has no distinct
        // "failed" mark (`readme.md`'s Iconography table: only `●` done /
        // `◐` running / `○` pending / `✔` accepted — "if a mark is needed
        // and it is not in that table, do not draw one") — an error keeps
        // the `●` done glyph but in `del` (red) instead of `add`, the same
        // "colour carries the meaning" rule the rest of this system leans
        // on. No label of its own — a tool-activity group continues
        // whichever turn's content column it renders under.
        LogEntry::ToolActivity { calls, .. } => {
            let inner_width = ctx.body().width as usize;
            let content: Vec<Line<'static>> = calls
                .iter()
                .map(|c| {
                    // `4a`: "glyph, 2 spaces, tool name padded to 6
                    // characters … then the target" — its own example is
                    // `◐  bash  cargo test…`. This row shipped a single
                    // space and no pad, putting the name on cell 15 and the
                    // target on 20 where the reference puts them on 16 and
                    // 22, so no two tool rows lined up with each other.
                    //
                    // The 6 is a minimum, not a width: a name of 6 or more
                    // characters would touch its own target, and two runs
                    // colliding inside the body column is this UI's
                    // recurring defect. Where the reference's arithmetic
                    // runs out, the glyph's own 2-space rhythm is what
                    // continues it.
                    //
                    // What is *in* the target field is still the call id
                    // rather than the file or command the reference shows —
                    // `ToolActivityEntry` carries `call_id`, `name` and
                    // `status` and nothing else, so the real target is not
                    // available to render. See the conformance spec; that
                    // half is a data source, not a layout fix.
                    let target = if c.name.is_empty() {
                        c.call_id.clone()
                    } else {
                        let field = 6.max(c.name.width() + 2);
                        format!("{:<field$}({})", c.name, c.call_id)
                    };
                    // A *running* call's name is `accent_text` in the
                    // reference ("`◐  bash  cargo test…`" — the live row is
                    // the one the eye should land on), a finished one's is
                    // ordinary `body`.
                    let (glyph, text_color, summary) = match &c.status {
                        ToolActivityStatus::Running => (Span::styled("◐  ", Style::default().fg(pal.glyph_running)), pal.accent_text, None),
                        ToolActivityStatus::Completed { is_error: false, summary } => (Span::styled("●  ", Style::default().fg(pal.glyph_done)), pal.body, Some(summary.clone())),
                        ToolActivityStatus::Completed { is_error: true, summary } => (Span::styled("●  ", Style::default().fg(pal.del)), pal.body, Some(summary.clone())),
                    };
                    let left = vec![glyph, Span::styled(target, Style::default().fg(text_color))];
                    let right = match summary {
                        Some(s) => vec![Span::styled(s, Style::default().fg(pal.dim))],
                        None => vec![],
                    };
                    justified_line(left, right, inner_width)
                })
                .collect();
            with_label_column(content, label("harness", pal.speaker_agent))
        }
        // No glyph and no dedicated "warning" colour — the design system
        // has neither, and its palette has nothing named for a transient
        // retry. `label` keeps it a quiet, informational fact rather than
        // inventing a colour outside that fixed vocabulary.
        LogEntry::RetryAttempt { info } => {
            let status = info.status.map(|s| s.to_string()).unwrap_or_else(|| "-".to_string());
            body_lines(
                vec![
                    Span::styled("retry ", Style::default().fg(pal.label).add_modifier(Modifier::BOLD)),
                    Span::styled(format!("{} · attempt {} · {status}: {}", info.provider, info.attempt, info.message), Style::default().fg(pal.dim)),
                ],
                None,
                ctx,
            )
        }
        // While pending (`resolution: None`), a decision renders nothing at
        // all here — the decision panel (`super::decision`, a fixed
        // full-width band above the input) is the only place an unresolved
        // request is interactive, per the design system's own "Permission
        // prompt" screen. Once resolved it still renders here: the log is
        // the permanent record.
        //
        // As a *record* it is a tool call the developer let through (or
        // stopped), and the reference already has a shape for that —
        // `ToolLine.jsx`, on the turn's own body column. It used to reuse
        // the panel's card instead, so an answered prompt left a full-width
        // `bar`-filled block sitting in the middle of the conversation,
        // aligned to nothing around it, with the raw `PromptResponse` debug
        // string underneath: "the following chat rows do not match the
        // designs at all and are all misaligned and wonky."
        LogEntry::PermissionPrompt { payload, resolution: Some(resolved), .. } => {
            let (kind, target) = payload_call(payload);
            let summary = Span::styled(resolved.label.clone(), Style::default().fg(if resolved.allowed { pal.dim } else { pal.del }));
            with_label_column(vec![tool_line(&kind, &target, resolved.allowed, vec![summary], ctx)], label("harness", pal.speaker_agent))
        }
        // An answered Edit gets the same tool line, over the diff it was
        // answering — `Turn.jsx`'s own `write src/gateway/limit.rs  +84`
        // row followed by an `InlineDiff.jsx` box, which is exactly what
        // this entry has to show. The right-flush summary is the diff stat
        // for an approved edit and the refusal for a denied one, since a
        // denied edit's `+n -m` would describe a change that never happened.
        LogEntry::ApprovalCard { diff, resolution: Some(approved), .. } => {
            let (path, body) = diff::parse_body(diff);
            let body = diff::number_lines(body);
            let summary = if *approved { diff::stat_spans(&body, ctx) } else { vec![Span::styled("denied", Style::default().fg(pal.del))] };
            let mut content = vec![tool_line("edit", &path.map(|p| diff::strip_prefix(&p)).unwrap_or_default(), *approved, summary, ctx)];
            if *approved {
                content.extend(diff::boxed(&body, diff::Budget { collapse_context: true, max_rows: None }, Row::field(pal.diff_box), ctx.body()));
            }
            with_label_column(content, label("harness", pal.speaker_agent))
        }
        LogEntry::ApprovalCard { resolution: None, .. } | LogEntry::PermissionPrompt { resolution: None, .. } => Vec::new(),
        LogEntry::TurnEnded { reason } => {
            use crate::log::TurnEndReasonKind;
            let spans = match reason {
                TurnEndReasonKind::EndTurn => return vec![],
                TurnEndReasonKind::Cancelled => vec![Span::styled("— turn cancelled —", Style::default().fg(pal.dim))],
                TurnEndReasonKind::Error(message) => vec![Span::styled(format!("— turn ended in error: {message} —"), Style::default().fg(pal.dim))],
            };
            body_lines(spans, None, ctx)
        }
        LogEntry::Error { message } => body_lines(
            vec![
                Span::styled("error: ", Style::default().fg(pal.del).add_modifier(Modifier::BOLD)),
                Span::styled(message.clone(), Style::default().fg(pal.del)),
            ],
            None,
            ctx,
        ),
        LogEntry::Notice { message } => body_lines(
            vec![Span::styled("notice: ", Style::default().fg(pal.quiet)), Span::styled(message.clone(), Style::default().fg(pal.dim))],
            None,
            ctx,
        ),
    }
}

/// `ToolLine.jsx`, exactly as the reference lays it out: the status glyph,
/// two spaces, the tool name in a 6-cell column (`read  `, `write `,
/// `edit  `, `bash  `), then the target, with `summary` flush to the body
/// column's right edge.
///
/// The target is elided rather than wrapped. This row is composed by hand
/// on a right-flush layout, so a target wider than the column left would
/// wrap under `Paragraph`'s own wrapper — which knows nothing about the
/// label column already applied — and strand its tail against the frame's
/// left edge, the failure this module's own doc comment describes. A shell
/// command is arbitrarily long and completely ordinary input, so this is
/// the common case, not the exotic one.
fn tool_line(kind: &str, target: &str, ok: bool, summary: Vec<Span<'static>>, ctx: Ctx) -> Line<'static> {
    /// `read  ` / `write ` / `edit  ` / `bash  ` — the reference pads every
    /// tool name into the same column so the targets line up under one
    /// another.
    const NAME_COL: usize = 6;
    let pal = ctx.pal;
    let width = ctx.body().width as usize;
    let summary_width: usize = summary.iter().map(|s| s.content.width()).sum();
    // Two cells of gap between the target and the summary at minimum, so
    // the two never read as one string.
    let room = width.saturating_sub(2 + NAME_COL).saturating_sub(summary_width).saturating_sub(2);
    let left = vec![
        Span::styled("●  ", Style::default().fg(if ok { pal.glyph_done } else { pal.del })),
        Span::styled(format!("{kind:<NAME_COL$}"), Style::default().fg(pal.label)),
        Span::styled(elide(target, room), Style::default().fg(pal.text)),
    ];
    justified_line(left, summary, width)
}

/// The `kind` and `target` a `PromptPayload` was asking about — the same
/// two facts the decision panel's own `PromptView` reads off it, in the
/// shape a tool line wants them.
fn payload_call(payload: &PromptPayload) -> (String, String) {
    match payload {
        PromptPayload::Tool { kind, target, .. } => (kind.clone(), target.clone()),
        PromptPayload::ContextFile { path } => ("context".into(), path.display().to_string()),
        PromptPayload::Edit { kind } => ("edit".into(), kind.clone()),
    }
}

/// Builds the `harness` turn's content — never indented itself; the caller
/// lays the whole result out under the label column via
/// `with_label_column`, so every box built here (diff, code block) sizes
/// itself against the body column, not the full panel width, or it would
/// overflow past the right edge once that column is added back.
fn render_assistant_text(text: &str, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    let body = ctx.body();
    let mut lines: Vec<Line<'static>> = Vec::new();
    for segment in markdown::split_code_fences(text) {
        match segment {
            // No fill of its own — `Prose.jsx` is plain coloured text on
            // the panel ground, no background. Handed over whole rather
            // than a line at a time: a table's columns are sized against
            // every row of it at once, so the block pass lives in
            // `markdown` and this arm no longer wraps (it does its own).
            Segment::Prose(s) => lines.extend(markdown::render_prose(&s, body)),
            // A fenced ```diff block gets the same full-width red/green
            // per-line treatment (line-number gutter included) as the Edit
            // approval card, instead of the generic code-block box below —
            // the diff renderer already exists precisely for "show a diff"
            // (`InlineDiff.jsx`'s own job), so this reuses it rather than
            // inventing a second diff presentation. No row budget: the log
            // scrolls, so nothing here has to fit a fixed band.
            Segment::Code { lang, body: diff_text } if lang.eq_ignore_ascii_case("diff") => {
                let (_, parsed) = diff::parse_body(&diff_text);
                lines.extend(diff::boxed(&diff::number_lines(parsed), diff::Budget::default(), Row::field(pal.diff_box), body));
            }
            // A real code-block box, with a dim language label instead of
            // the fence's own literal ` ``` ` markers, on `diff_box` — the
            // design system's one nested-quote surface, already carrying
            // the inline diff for the same reason (a quoted block inside
            // prose). `highlight_lines` builds its syntect theme from this
            // theme's palette — the five `--tui-syn-*` roles — so the code
            // on this surface follows the palette like every other cell.
            //
            // Built from the same `Row::field` the diff uses. The two are
            // the *same* component in the design system — one quoted block
            // on one nested surface — and they must not diverge again: this
            // one once rendered as a bare field while the diff had a drawn
            // box, so a code fence and a diff fence in the same reply read
            // as two unrelated treatments ("the diff boxes in the chat ...
            // are completely different from the diff box seen in the
            // permissions dialog"). Now neither is stroked and both are the
            // same recessed field, so they agree by construction.
            //
            // The blank first and last rows are the field's own: they give
            // the step off the surrounding ground a full row to read
            // against at the top and bottom, which is what the drawn edge
            // used to do.
            Segment::Code { lang, body: code } => {
                let row = Row::field(pal.diff_box).pad(1);
                let label = if lang.is_empty() { "code".to_string() } else { lang.clone() };
                lines.extend(row.text(&label, pal.label, body));
                lines.push(row.blank(body));
                for code_line in highlight::highlight_lines(&lang, &code, pal.theme) {
                    let spans: Vec<Span<'static>> = code_line.into_iter().map(|s| Span::styled(s.content, s.style.bg(pal.diff_box))).collect();
                    lines.extend(row.build(spans, body));
                }
                lines.push(row.blank(body));
            }
        }
    }
    lines
}

/// Mirrors mjolnir-cli's own `/`-prefix check (`text.trim_start().strip_prefix('/')`
/// in `slash.rs`) — this crate can't depend on that one to reuse it
/// directly (cli depends on tui, not the other way around), so the rule is
/// duplicated; keep the two in sync if it ever changes.
fn is_command(text: &str) -> bool {
    text.trim_start().starts_with('/')
}

/// The empty state — the design system's `14d`, and the screen a returning
/// developer actually opens into: what `mjolnir` shows in a repository it
/// has been pointed at before, and what `/clear` leaves behind.
///
/// The wordmark leads, because "the mark identifies a frame with no
/// transcript to identify it" — this and first run are the only two places
/// it appears. Then three facts on the ordinary 8-cell label column, so the
/// empty frame lines up on the same edge the transcript will use the moment
/// there is one. Then the one line saying what to do next.
///
/// # What is not here
///
/// Turn 14 took the build's **version and commit** off this screen. The
/// version lives in first run's top bar and the session's top bar carries
/// the model instead, so neither fact is lost — but neither belongs in the
/// resting state either. (`tests::the_top_bar_reports_the_running_builds_version`
/// still pins the version; the commit's own assertion went with this
/// change, and `version.rs` pins the constant itself.)
///
/// Two of `14d`'s own values are **not fabricated**, the same call
/// `chrome::draw_top_bar` makes about the reference's context gauge and
/// session cost:
///
/// * the `in` row shows the working directory alone — the reference adds a
///   git branch and a dirty marker, and nothing in `StatusInfo` tracks one;
/// * the `access` row shows the three permission states this directory
///   actually has, rather than the reference's single tier word. A tier is
///   what first run *writes*; it is not what is stored, and a
///   `permissions.yaml` edited by hand need not correspond to any tier at
///   all. Reporting one would be a guess printed as a fact on the screen
///   whose whole job is to say what this directory permits.
pub(super) fn intro_content(app: &App, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    let status = &app.status;

    // One fact row: its name in the 8-cell label column, its value on the
    // body column. The same helper every transcript turn uses, so the empty
    // frame and the first turn drawn into it share one edge.
    let field = |name: &str, spans: Vec<Span<'static>>| -> Line<'static> {
        with_label_column(vec![Line::from(spans)], Some((name, pal.label))).remove(0)
    };

    let provider = match app.current_provider.as_deref() {
        // `anthropic · claude-sonnet-5` — one group, two facts, so ` · `
        // parts them rather than the 6-cell gap that parts groups.
        Some(id) => vec![
            Span::styled(id.to_string(), Style::default().fg(pal.value)),
            Span::styled(" · ".to_string(), Style::default().fg(pal.dim)),
            Span::styled(status.model_name.clone(), Style::default().fg(pal.value)),
        ],
        // A hand-written endpoint the catalogue cannot name is a real
        // configuration, not an error — the model still names itself.
        None => vec![Span::styled(status.model_name.clone(), Style::default().fg(pal.value))],
    };

    let mut access = Vec::new();
    for (i, (name, state)) in [("read", status.read), ("shell", status.shell), ("edit", status.edit)].into_iter().enumerate() {
        if i > 0 {
            access.push(Span::raw("  "));
        }
        access.extend(access_spans(name, state, ctx));
    }

    let mut content: Vec<Line<'static>> = Vec::with_capacity(INTRO_ROWS);
    content.push(super::first_run::wordmark(ctx));
    content.push(Line::default());
    // Elided, not wrapped — `INTRO_ROWS` below promises one row per fact,
    // and a deep checkout is the one value here that routinely outruns the
    // body column.
    let cwd = elide(&status.cwd.clone().unwrap_or_default(), ctx.body().width as usize);
    content.push(field("in", vec![Span::styled(cwd, Style::default().fg(pal.value))]));
    content.push(field("provider", provider));
    content.push(field("access", access));
    content.push(Line::default());
    content.push(Line::from(vec![
        Span::raw(" ".repeat(MARGIN_X)),
        Span::styled("Ask for a change, or ", Style::default().fg(pal.dim)),
        Span::styled("/", Style::default().fg(pal.quiet)),
        Span::styled(" for commands.", Style::default().fg(pal.dim)),
    ]));
    debug_assert_eq!(content.len(), INTRO_ROWS, "INTRO_ROWS must match what intro_content builds");
    content
}

/// Rows [`intro_content`] always renders. Fixed, not derived: every row is
/// one line whatever the model name or directory is, since each is elided
/// rather than wrapped.
///
/// Was 8 until the transcript band got its own blank row at each end
/// (`super::LOG_PAD_ROWS`). The eighth was this screen's own trailing gap to
/// the composer, hand-rolled here because nothing else provided one; the
/// band now spaces *every* transcript off the bars, so keeping it too put
/// two blank rows under the hero where `14d` has one.
pub(super) const INTRO_ROWS: usize = 7;

/// One `label: state` pair in the hero's access row. No filled chip — the
/// design system's own rule is that the accent is "a mark or a line, never
/// a filled field," and none of its components use a background-filled
/// badge for a state word.
///
/// Both words are `value`, and the **word** is what distinguishes them.
/// They used to be `add`/`del` — green and red — on the reasoning that the
/// pair "already reads as allow/deny at a glance." It does, but at a price
/// the system does not sell: those are the two diff hues, and rule 1 of
/// three is that "nothing in a frame is a foreign colour. The only
/// exceptions are the two diff hues, 148° and 25°" — exceptions *for the
/// diff*, because they have to be unmistakably not-the-accent. Spent
/// anywhere else they stop meaning "changed line": a screenshot judge read
/// this row as deleted lines, and in the light theme `deny` at #b0122e was
/// the most saturated thing in a deliberately shallow frame.
///
/// `value` is not a guess: `--tui-value` is the role named for "right-flush
/// facts and permission \"off\" values", which is literally this. It also
/// makes the row consistent with its own siblings — `in` and `provider`
/// two rows up are already `value`, so the hero now reads as one key/value
/// block instead of two rows of facts and one of signals.
fn access_spans(label: &str, state: PermState, ctx: Ctx) -> Vec<Span<'static>> {
    let word = match state {
        PermState::Allowed => "allow",
        PermState::Denied => "deny",
    };
    vec![
        Span::styled(format!("{label}:"), Style::default().fg(ctx.pal.label)),
        Span::styled(format!("{word} "), Style::default().fg(ctx.pal.value)),
    ]
}

/// Pushes [`intro_content`] to the *bottom* of the log panel's inner
/// `height`, against the composer.
///
/// It used to be vertically centred. `14d`'s body band is
/// `justify-content: flex-end`, like the transcript's own — which is the
/// point: the empty state sits exactly where the first turn will appear, so
/// typing into the composer does not make the screen jump. A centred hero
/// had the facts drift upward as the terminal grew.
///
/// On a terminal too short for the content, `pad_top` saturates to 0 and it
/// starts at the top and scrolls like any other tall log content would.
fn hero_lines(app: &App, height: u16, ctx: Ctx) -> Vec<Line<'static>> {
    let content = intro_content(app, ctx);
    let pad_top = (height as usize).saturating_sub(content.len());
    let mut lines = Vec::with_capacity(pad_top + content.len());
    lines.extend(std::iter::repeat_with(Line::default).take(pad_top));
    lines.extend(content);
    lines
}
