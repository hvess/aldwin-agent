//! The conversation log: one `LogEntry` at a time turned into rows, the
//! welcome hero that stands in for an empty log, and the panel that scrolls
//! them.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph, Wrap};
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
/// be the inner, not outer, rect). Shared by [`draw_log`] (renders it) and
/// [`row_count`] (counts its wrapped rows for scroll math), so the two can
/// never disagree about what the log contains. An empty log shows the
/// welcome hero instead of any entries — the two are mutually exclusive, so
/// there's no "separate the banner from the first real entry" case.
fn build_lines(app: &App, ctx: Ctx, height: u16) -> Vec<Line<'static>> {
    if app.log.is_empty() {
        return hero_lines(app, height, ctx);
    }
    let mut lines: Vec<Line> = Vec::new();
    for entry in app.log.iter() {
        // An entry can render to nothing at all (a routine `TurnEnded` is
        // folded into the status line's own activity indicator instead of
        // getting its own log row), so the blank separator is keyed on
        // whether anything has actually been pushed yet, not on the entry's
        // index — otherwise a silent entry would still claim a blank row.
        let rendered = render_entry(entry, ctx);
        if rendered.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            // A fresh `UserMessage`/`AssistantText` starts a new
            // conversational turn and gets a turn break between it and
            // whatever came before. That break is a *band*, not a rule: the
            // design system's Turn 13 rebuild replaced every freestanding
            // rule with "one full-width row of the composer's tone", and
            // says of the terminal case that "in a terminal that is a
            // single `Style::bg` on a one-row rect, so nothing here needs
            // approximating". So this is a row of `break_` with no glyph in
            // it at all — the tonal step off the transcript ground is the
            // whole separator.
            //
            // Tool activity/retry/error/notice entries continue the current
            // turn rather than starting a new one, so they only get the
            // plain blank row a turn's own internal groups get.
            if matches!(entry, LogEntry::UserMessage { .. } | LogEntry::AssistantText { .. }) {
                lines.push(Line::default());
                lines.push(band_row(ctx.pal.break_, ctx));
                lines.push(Line::default());
            } else {
                lines.push(Line::default());
            }
        }
        lines.extend(rendered);
    }
    // No spinner row is appended here — per explicit developer feedback, an
    // active turn used to get an animated "thinking…"/"working…" row both
    // here (trailing the log) *and* in the status line right above the
    // input, which read as a plain duplicate of the same information. The
    // status line is now the one place live turn activity shows.
    lines
}

/// Renders the log panel: `block` onto `outer`, its content onto `inner`
/// (already computed once by `super::draw`).
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
pub(super) fn draw_log(frame: &mut Frame, outer: Rect, inner: Rect, block: Block<'static>, app: &App) {
    let ctx = Ctx::new(app.theme.palette(), inner.width);
    let lines = build_lines(app, ctx, inner.height);
    // `scroll.offset` is in *wrapped screen rows* (see [`row_count`]), so it
    // must go through `Paragraph::scroll`, which advances the same wrapping
    // line-composer `Paragraph::line_count` uses internally — not a
    // `.skip()` on `lines` beforehand, which would count in logical
    // (pre-wrap) rows instead and drift out of sync the moment anything
    // wraps.
    let offset = app.scroll.offset.min(u16::MAX as usize) as u16;

    frame.render_widget(block, outer);
    frame.render_widget(Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }).scroll((offset, 0)), inner);
}

/// The number of terminal rows the log panel's inner area needs to fully
/// render at `width` × `height` — what `ScrollState` actually compares
/// against viewport height, wrapping included. Delegates to ratatui's own
/// `Paragraph::line_count`, which runs the exact same word-wrapper
/// [`draw_log`]'s render path uses, rather than re-deriving wrap behaviour
/// by hand — see mjolnir-tui.md's 2026-08-29 scrolling-fix and
/// wrapped-row-scroll-math Progress notes for the two incidents this
/// discipline exists to prevent from recurring. `height` only matters for
/// the empty-log hero path (it bottom-anchors the hero); the non-empty
/// path's row count is width-only.
pub(crate) fn row_count(app: &App, width: u16, height: u16) -> usize {
    let lines = build_lines(app, Ctx::new(app.theme.palette(), width), height);
    Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }).line_count(width)
}

fn render_entry(entry: &LogEntry, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
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
                return text.lines().map(|l| Line::from(Span::styled(format!("> {l}"), Style::default().fg(pal.dim)))).collect();
            }
            let style = Style::default().fg(pal.text);
            let content: Vec<Line<'static>> =
                text.lines().flat_map(|l| wrap_line(Line::from(Span::styled(l.to_string(), style)), ctx.body().width as usize)).collect();
            with_label_column(content, Some(("you", pal.speaker_you)))
        }
        LogEntry::AssistantText { text } => with_label_column(render_assistant_text(text, ctx), Some(("harness", pal.speaker_agent))),
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
                    let target = if c.name.is_empty() { c.call_id.clone() } else { format!("{} ({})", c.name, c.call_id) };
                    // A *running* call's name is `accent_text` in the
                    // reference ("`◐  bash  cargo test…`" — the live row is
                    // the one the eye should land on), a finished one's is
                    // ordinary `body`.
                    let (glyph, text_color, summary) = match &c.status {
                        ToolActivityStatus::Running => (Span::styled("◐ ", Style::default().fg(pal.glyph_running)), pal.accent_text, None),
                        ToolActivityStatus::Completed { is_error: false, summary } => (Span::styled("● ", Style::default().fg(pal.glyph_done)), pal.body, Some(summary.clone())),
                        ToolActivityStatus::Completed { is_error: true, summary } => (Span::styled("● ", Style::default().fg(pal.del)), pal.body, Some(summary.clone())),
                    };
                    let left = vec![glyph, Span::styled(target, Style::default().fg(text_color))];
                    let right = match summary {
                        Some(s) => vec![Span::styled(s, Style::default().fg(pal.dim))],
                        None => vec![],
                    };
                    justified_line(left, right, inner_width)
                })
                .collect();
            with_label_column(content, None)
        }
        // No glyph and no dedicated "warning" colour — the design system
        // has neither, and its palette has nothing named for a transient
        // retry. `label` keeps it a quiet, informational fact rather than
        // inventing a colour outside that fixed vocabulary.
        LogEntry::RetryAttempt { info } => {
            let status = info.status.map(|s| s.to_string()).unwrap_or_else(|| "-".to_string());
            let line = Line::from(vec![
                Span::styled("retry ", Style::default().fg(pal.label).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{} · attempt {} · {status}: {}", info.provider, info.attempt, info.message), Style::default().fg(pal.dim)),
            ]);
            with_label_column(vec![line], None)
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
            with_label_column(vec![tool_line(&kind, &target, resolved.allowed, vec![summary], ctx)], None)
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
            with_label_column(content, None)
        }
        LogEntry::ApprovalCard { resolution: None, .. } | LogEntry::PermissionPrompt { resolution: None, .. } => Vec::new(),
        LogEntry::TurnEnded { reason } => {
            use crate::log::TurnEndReasonKind;
            let line = match reason {
                TurnEndReasonKind::EndTurn => return vec![],
                TurnEndReasonKind::Cancelled => Line::from(Span::styled("— turn cancelled —", Style::default().fg(pal.dim))),
                TurnEndReasonKind::Error(message) => Line::from(Span::styled(format!("— turn ended in error: {message} —"), Style::default().fg(pal.dim))),
            };
            with_label_column(vec![line], None)
        }
        LogEntry::Error { message } => with_label_column(
            vec![Line::from(vec![
                Span::styled("error: ", Style::default().fg(pal.del).add_modifier(Modifier::BOLD)),
                Span::styled(message.clone(), Style::default().fg(pal.del)),
            ])],
            None,
        ),
        LogEntry::Notice { message } => with_label_column(
            vec![Line::from(vec![Span::styled("notice: ", Style::default().fg(pal.quiet)), Span::styled(message.clone(), Style::default().fg(pal.dim))])],
            None,
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
            // the panel ground, no background.
            Segment::Prose(s) => lines.extend(s.lines().flat_map(|l| wrap_line(markdown::render_line(l, body), body.width as usize))),
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
            // prose). `highlight_lines` picks the matching half of the
            // `base16-ocean` pair from the app theme, so the surface
            // follows the palette like every other one.
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
    content.push(field("in", vec![Span::styled(status.cwd.clone().unwrap_or_default(), Style::default().fg(pal.value))]));
    content.push(field("provider", provider));
    content.push(field("access", access));
    content.push(Line::default());
    content.push(Line::from(vec![
        Span::raw(" ".repeat(MARGIN_X)),
        Span::styled("Ask for a change, or ", Style::default().fg(pal.dim)),
        Span::styled("/", Style::default().fg(pal.quiet)),
        Span::styled(" for commands.", Style::default().fg(pal.dim)),
    ]));
    content.push(Line::default());
    debug_assert_eq!(content.len(), INTRO_ROWS, "INTRO_ROWS must match what intro_content builds");
    content
}

/// Rows [`intro_content`] always renders. Fixed, not derived: every row is
/// one line whatever the model name or directory is, since each is elided
/// or simply allowed to run to the frame's edge rather than wrapped.
pub(super) const INTRO_ROWS: usize = 8;

/// One `label: state` pair in the hero's access row. No filled chip — the
/// design system's own rule is that the accent is "a mark or a line, never
/// a filled field," and none of its components use a background-filled
/// badge for a state word; add/del (green/red) plain text already reads as
/// allow/deny at a glance.
fn access_spans(label: &str, state: PermState, ctx: Ctx) -> Vec<Span<'static>> {
    let (word, word_fg) = match state {
        PermState::Allowed => ("allow", ctx.pal.add),
        PermState::Denied => ("deny", ctx.pal.del),
    };
    vec![Span::styled(format!("{label}:"), Style::default().fg(ctx.pal.label)), Span::styled(format!("{word} "), Style::default().fg(word_fg))]
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
