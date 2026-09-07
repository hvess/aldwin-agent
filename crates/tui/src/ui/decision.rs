//! The decision panel: the one interactive surface for a pending Edit
//! approval or permission prompt, plus the resolved cards those leave
//! behind in the log.
//!
//! Structure, top to bottom, matching the design system's Permission
//! screen: a title band, the card body (a sentence, the target, and either
//! a recessed diff field or a command block), the grant the answer would
//! save, a separator band, the numbered options, and a key-hint footer.

use mjolnir_permissions::PromptPayload;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::diff;
use super::grid::{elide, Ctx, MARGIN_X};
use super::row::{band_row, Row};
use crate::app::{App, GrantSummary, GrantUnit, PatternScope, PendingFront};
use crate::palette::Palette;

/// Maximum rows the panel is allowed to claim, derived from the frame's
/// total height rather than fixed — reserves room for at least one row of
/// the conversation log and the bars above and below it, so an unusually
/// large diff can never push the rest of the UI off-frame the way an
/// unbounded `Constraint::Length` could. Floors at 6 (enough for a short
/// title/keys/padding-only panel) even on a terminal too short to honour
/// the reservation in full — a degenerate case, not one worth failing
/// gracefully out of.
pub(super) fn max_height(frame_height: u16) -> usize {
    // While a decision is pending the panel *is* the bottom bar — it takes
    // the composer's and status line's rows rather than stacking above
    // them (see `super::draw`), so those aren't reserved here.
    // The top bar's rule row and the panel's own edge row are both gone —
    // neither the bar nor the panel is stroked any more, so neither spends
    // a row on an edge.
    const RESERVED_FOR_REST_OF_UI: u16 = 3 /* top bar */ + 1 /* one row of log */;
    // [`panel_lines`] adds the band's row and the footer's rows *outside*
    // the budget this bounds (see there for why) — reserved here too, so
    // the combined total still fits the same overall budget, not just the
    // clamped body alone.
    const PANEL_CHROME: u16 = 1 /* band */ + 2 /* footer padding + hint */;
    (frame_height.saturating_sub(RESERVED_FOR_REST_OF_UI + PANEL_CHROME) as usize).max(6)
}

/// Wrapped-row count of `lines` at `width`, measured with the same
/// `Paragraph::line_count` that [`draw_panel`]'s own `Wrap` composes with, so
/// the height `super::draw` reserves and what actually renders can never
/// disagree.
///
/// The transcript used to be counted this way too and no longer is — it
/// pre-wraps instead, so its rows *are* screen rows (see
/// `super::transcript::rows`). The panel keeps the two-pass shape on purpose:
/// it is a bounded band rebuilt only while a decision is open, so the second
/// wrap costs nothing measurable, and `clamp_panel`'s budget arithmetic is
/// written against a wrapping `Paragraph`.
pub(super) fn row_count(lines: &[Line<'static>], width: u16) -> usize {
    Paragraph::new(Text::from(lines.to_vec())).wrap(Wrap { trim: false }).line_count(width)
}

pub(super) fn draw_panel(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>, pal: &Palette) {
    if area.height == 0 {
        return;
    }
    // The panel's own surface, painted before its content. `Row` already
    // fills every row it builds, so this changes no pixel a correct panel
    // draws — it changes what a *hole* in one shows. Anything that ever
    // fails to carry a fill now falls through to the panel's `bar` rather
    // than to the frame's `ground`, which is what the canvas underneath
    // this area actually holds, and which read as a near-black gap in the
    // dark theme and a white one in the light.
    frame.render_widget(Block::new().style(Style::default().bg(pal.bar)), area);
    // `Wrap { trim: false }` — matches [`row_count`]'s own wrap mode, so
    // the precomputed panel height and what actually renders here can never
    // desync.
    frame.render_widget(Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }), area);
}

/// Everything the panel needs to say about one pending payload, derived in
/// one place.
///
/// The four facts below used to be four separate `match`es over
/// `PromptPayload` scattered across the panel (the humanized sentence, the
/// literal call, the band's badge, and whether the target renders as a
/// command block), so adding a payload variant meant finding all four. It
/// is one arm here instead.
struct PromptView {
    /// A plain-English sentence naming what's actually being asked — the
    /// row a developer reads first to decide. The old title *was* the raw
    /// tool call, with nothing telling a developer what that call does at a
    /// glance. "The agent," not "Claude" — `readme.md`'s Content
    /// Fundamentals: third person for the model when the harness speaks
    /// about it.
    sentence: String,
    /// The literal `kind: target` the sentence above is describing —
    /// unchanged in substance from the old title, just demoted to a dim
    /// subtitle now that the title itself carries the explanation.
    call:     String,
    /// The title band's right-aligned badge — `Modal.jsx`'s `badge` prop.
    badge:    String,
    /// `Some(command)` when the target is a shell command and should get
    /// `CommandBlock.jsx`'s own treatment. Other kinds (a file path, a
    /// context-file load) show their exact target as plain label text
    /// instead (`readme.md`'s "Targets are exact" rule); a `$` prompt only
    /// means something for an actual shell command.
    command:  Option<String>,
}

impl PromptView {
    fn of(payload: &PromptPayload) -> Self {
        match payload {
            PromptPayload::Tool { kind, target, .. } => Self {
                sentence: match kind.as_str() {
                    "read" => "The agent wants to read a file".into(),
                    "shell" => "The agent wants to run a shell command".into(),
                    "explain" => "The agent wants to inspect code".into(),
                    other => format!("The agent wants to use \"{other}\""),
                },
                call:     format!("{kind}: {target}"),
                badge:    kind.clone(),
                command:  (kind == "shell").then(|| target.clone()),
            },
            PromptPayload::ContextFile { path } => Self {
                sentence: format!("The agent wants to load {} as context", path.display()),
                call:     format!("context_file: {}", path.display()),
                badge:    "context".into(),
                command:  None,
            },
            // Never actually reaches this card in production — `App::
            // decision_options`' Edit arm returns no options, since Edit
            // uses the separate ToolApprovalRequested/ApprovalCard path
            // instead (mjolnir-permissions.md's Edit Exception). Kept for a
            // complete, non-panicking match, not a live UI path.
            PromptPayload::Edit { kind } => {
                Self { sentence: "The agent wants to edit a file".into(), call: format!("edit: {kind}"), badge: "edit".into(), command: None }
            }
        }
    }
}

/// The Edit approval card: a sentence, the path, and the diff as a
/// recessed field on `diff_box` — a step off the card's `bar`, so it reads
/// as "a quoted block inside this card," the same nesting the command block
/// uses for a different payload kind. No outline: see the note on borders
/// in `super`.
///
/// Only ever the live panel — a *resolved* Edit's record in the log is a
/// tool line plus its diff field on the turn's own body column, not a
/// second copy of this card (see `transcript::render_entry`).
///
/// `card_rows` caps the whole card — [`panel_lines`] passes what's left of
/// the panel's budget once the tail is known, and the diff field is sized
/// against whatever the card's own (wrappable, so measured rather than
/// assumed) head leaves of that, so the card always fits its budget.
fn approval_card(diff_text: &str, tail: Vec<Line<'static>>, card_rows: Option<usize>, ctx: Ctx) -> Vec<Line<'static>> {
    /// One diff row and one elision marker — below this a quoted field
    /// can't say anything a plain note wouldn't say better. It was 4 while
    /// the field had two edge rows of its own to pay for.
    const MIN_BOX_ROWS: usize = 2;

    let pal = ctx.pal;
    let card = Row::card(pal.bar);
    let (path, body) = diff::parse_body(diff_text);
    let body = diff::number_lines(body);

    // A blank filled row top and bottom — plain terminal text sat flush
    // against the card's edges, which read as cramped next to the
    // reference's generous interior padding. Plain sentence in `body`, not
    // bold and not accent: the title band above already carries the accent
    // weight this row doesn't need to repeat.
    let mut lines = vec![card.blank(ctx)];
    lines.extend(card.text("The agent wants to edit this file.", pal.body, ctx));
    if let Some(path) = path {
        lines.extend(card.text(&diff::strip_prefix(&path), pal.label, ctx));
    }
    lines.push(card.blank(ctx));

    let room = card_rows.map(|max| max.saturating_sub(lines.len()));
    if room.is_some_and(|room| room < MIN_BOX_ROWS) {
        // Too short a panel to draw a box that could say anything. A plain
        // note naming what was left out is honest about that; half a box —
        // which is what a blind row-budget cut produced before — is not.
        // Kept short deliberately: this row only ever appears on a panel
        // already too cramped to spare a second one, and a note that wraps
        // is a note `clamp_panel` then has to throw away.
        let hidden = body.len();
        lines.extend(card.text(&format!("⋯ {hidden} diff line{} not shown ⋯", if hidden == 1 { "" } else { "s" }), pal.dim, ctx));
    } else {
        let budget = diff::Budget { collapse_context: true, max_rows: room };
        lines.extend(diff::boxed(&body, budget, Row::field(pal.diff_box).inset(MARGIN_X, pal.bar), ctx));
    }

    lines.extend(tail);
    lines
}

/// The permission-prompt card — the live panel's body for a `PromptPayload`,
/// with the same `tail` contract as [`approval_card`].
fn prompt_card(payload: &PromptPayload, tail: Vec<Line<'static>>, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    let card = Row::card(pal.bar);
    let view = PromptView::of(payload);

    let mut lines = vec![card.blank(ctx)];
    lines.extend(card.text(&view.sentence, pal.body, ctx));
    match &view.command {
        Some(command) => {
            lines.push(card.blank(ctx));
            lines.extend(command_block(command, ctx));
        }
        None => lines.extend(card.text(&view.call, pal.label, ctx)),
    }
    lines.extend(tail);
    lines
}

/// `CommandBlock.jsx`: a `ground`-coloured field, *inset* from the card's
/// own edges, with the command prefixed by an accent `$`.
///
/// The inset is the whole point of the component and it was missing: the
/// reference wraps the field in the card's `padding: 0 27px` and gives the
/// field its own `padding-left: 18px` on top, so the block reads as a
/// quoted object sitting inside the card with `bar` visible down both
/// sides, and the `$` lands on cell 5. Built without the margin it instead
/// ran the full width of the panel — "the command row is not a box like in
/// the design but instead completely fills the entire dialog edge-to-edge
/// with no margin."
fn command_block(command: &str, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    let row = Row::card(pal.ground).inset(MARGIN_X, pal.bar).pad(COMMAND_BLOCK_PAD);
    let spans = vec![Span::styled("$ ", Style::default().fg(pal.speaker_you)), Span::styled(command.to_string(), Style::default().fg(pal.text))];
    let mut lines = vec![row.blank(ctx)];
    lines.extend(row.build(spans, ctx));
    lines.push(row.blank(ctx));
    lines
}

/// `CommandBlock.jsx`'s own `padding-left: 18px` — 2 cells inside the
/// field, on top of the `MARGIN_X` the field itself is inset by.
const COMMAND_BLOCK_PAD: usize = 2;

/// The panel's title row: a field of `panel_title` carrying the
/// plain-lowercase kind (`permission`) in `accent_text`, no glyph (per the
/// design system's revision log: a `▌` pip "indicated nothing" here), and
/// the payload's own kind right-aligned in `speaker_you`.
///
/// Two things here were wrong before and are fixed to match the system's
/// own Turn 13 corrections:
///
/// * The field was `band` — the *selection* colour, an accent fill. That is
///   precisely the treatment the design system rejected: a title row "began
///   on an accent field, which read as a filled accent band and broke the
///   guide's rule" that the accent is a mark and never a field. Its own
///   `_ds_bundle.js` had the identical bug ("`CommandPanel` painted its
///   title row with `--tui-band`, the selection colour, instead of
///   `--tui-panel-title`"). The title row is a *lift*: `panel_title` is the
///   top of the ground ladder, one step above the panel rather than an
///   accent laid over it, which leaves the selection band the only accent
///   fill in the frame besides the gauge.
/// * The badge was `hunk_header`, the gauge-fill accent step. On the
///   lighter title field that measured 2.2:1; the `you` step clears 3.8:1,
///   and the system moved the right-flush fact there for exactly that
///   reason.
///
/// No rule along the panel's top edge either — the step off the transcript
/// is the whole boundary (see `super::draw`).
pub(super) fn band(title: &str, badge: &str, ctx: Ctx) -> Line<'static> {
    Row::card(ctx.pal.panel_title).split(
        vec![Span::styled(title.to_string(), Style::default().fg(ctx.pal.accent_text))],
        vec![Span::styled(badge.to_string(), Style::default().fg(ctx.pal.speaker_you))],
        ctx,
    )
}

/// "↑↓ to move   1-N to pick   ⏎ to confirm" — `KeyHints.jsx`'s
/// key-coloured/verb-muted pair convention. Replaces the per-row shortcut
/// column the panel used to need, per the design system's revision log on
/// the permission screen: "the keys that were on the rows moved into the
/// footer."
pub(super) fn footer_hint(option_count: usize, ctx: Ctx) -> Vec<Span<'static>> {
    key_hints(&[("↑↓", "to move"), (&format!("1-{option_count}"), "to pick"), ("⏎", "to confirm")], ctx)
}

/// `KeyHints.jsx`'s pair convention itself: the key in the accent mark
/// colour, its verb muted, pairs parted by the `--group-gap` the bars use.
/// Shared so a second panel cannot invent a second spelling of the same
/// footer.
pub(super) fn key_hints(pairs: &[(&str, &str)], ctx: Ctx) -> Vec<Span<'static>> {
    const GROUP_GAP: usize = 6;
    let key = Style::default().fg(ctx.pal.mark);
    let verb = Style::default().fg(ctx.pal.quiet);
    let mut spans = Vec::new();
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" ".repeat(GROUP_GAP), verb));
        }
        spans.push(Span::styled(k.to_string(), key));
        spans.push(Span::styled(format!(" {v}"), verb));
    }
    spans
}

/// One row of a numbered list, reduced to what the row draws: the name and
/// the line beside it saying what picking it does. Both the permission
/// panel's decisions and the model picker's catalogue rows arrive here as
/// this, so the two lists cannot drift into two different controls.
pub(super) struct OptionRow {
    pub label:  String,
    pub detail: String,
}

/// The numbered, keyboard-navigable list of choices — one row per
/// `DecisionOption` — matching `OptionRow.jsx`'s selection convention: the
/// accent `▌` mark plus the `band` field together (never the mark alone),
/// the number in `accent_text` on the selected row and `label` otherwise.
///
/// Each row also carries the option's `detail`, dim, in a column aligned
/// across the whole list — what choosing this option concretely does
/// ("saved to .mjolnir/permissions.yaml"). The column is dropped wholesale
/// (never per-row, which would leave the list visibly ragged) on a frame
/// too narrow to seat it without wrapping every row: the labels alone still
/// resolve the list, and the panel body above already states the rule in
/// full.
pub(super) fn option_rows(options: &[OptionRow], selected: usize, ctx: Ctx) -> Vec<Line<'static>> {
    // Cells 0-5 are the mark and number columns (see the span layout
    // below); `DETAIL_GAP` parts the label column from the detail one, and
    // `MARGIN_X` keeps the longest detail off the frame's right edge.
    const DETAIL_GAP: usize = 3;
    const LABEL_COL: usize = 6;
    let pal = ctx.pal;
    let label_width = options.iter().map(|o| o.label.width()).max().unwrap_or(0);
    let detail_width = options.iter().map(|o| o.detail.width()).max().unwrap_or(0);
    let show_details = detail_width > 0 && LABEL_COL + label_width + DETAIL_GAP + detail_width + MARGIN_X <= ctx.width as usize;
    options
        .iter()
        .enumerate()
        .flat_map(|(i, opt)| {
            let is_selected = i == selected;
            let bg = if is_selected { pal.band } else { pal.bar };
            let mark_fg = if is_selected { pal.mark } else { pal.mark_idle };
            let number_fg = if is_selected { pal.accent_text } else { pal.label };
            let label_fg = if is_selected { pal.text } else { pal.body };
            // Flush to the frame's left edge — the one row type in the
            // system that skips `MARGIN_X` ("Four option rows, flush to the
            // frame's left edge like the command rows in 5c"). The exact
            // cell positions the reference lays out: `▌` in cell 0, two
            // spaces, the number in cell 3, two more spaces, then the label
            // starting in cell 6. No period after the number.
            let mut spans = vec![
                Span::styled("▌  ", Style::default().fg(mark_fg).bg(bg)),
                Span::styled(format!("{}  ", i + 1), Style::default().fg(number_fg).bg(bg)),
                Span::styled(opt.label.clone(), Style::default().fg(label_fg).bg(bg)),
            ];
            if show_details {
                let pad = label_width - opt.label.width() + DETAIL_GAP;
                spans.push(Span::styled(" ".repeat(pad), Style::default().bg(bg)));
                spans.push(Span::styled(opt.detail.clone(), Style::default().fg(pal.dim).bg(bg)));
            }
            Row::flush(bg).build(spans, ctx)
        })
        .collect()
}

/// How much of a grant rule [`grant_lines`] spells out before eliding. A
/// rule is `kind:pattern` over an arbitrary tool target, and an arbitrarily
/// long one is ordinary input (a long shell command); the target is already
/// shown in full in the card body directly above, so wrapping a second copy
/// of it across four rows would spend the panel's row budget repeating what
/// the developer just read. The head is what carries this line's meaning:
/// which kind, and that the pattern is the literal target rather than a
/// wildcard.
pub(super) const GRANT_RULE_MAX: usize = 56;

/// The panel's statement of what a *saved* answer would write — one line
/// naming the literal `kind:pattern` rule, plus a second naming the other
/// scope Tab would switch to when the target has one.
///
/// Both exist because of the same developer feedback: "permissions are not
/// clear, are we approving the tool? are we approving the directory? what
/// are we concretely doing." The tier labels can't answer that on their own
/// — they stay identical whichever pattern is selected — and the line they
/// replace only rendered for path-like targets, so a `shell` prompt said
/// nothing at all about whether "allow" meant this command or the shell
/// tool. Naming the rule verbatim answers it in the same vocabulary the
/// developer will later read back out of `permissions.yaml`.
fn grant_lines(grant: &GrantSummary) -> Vec<String> {
    let mut lines = vec![format!("saving an answer adds the rule  {}", elide(&grant.rule, GRANT_RULE_MAX))];
    if let Some(alternate) = &grant.alternate {
        let alternate = elide(alternate, GRANT_RULE_MAX);
        // The wording follows the tool's own broad unit (ADR 0001): a path
        // widens to its directory, a command to its program. One shared
        // phrasing was wrong for half the prompts as soon as `shell` stopped
        // granting the exact argv.
        lines.push(match (grant.scope, &grant.unit) {
            (PatternScope::Exact, GrantUnit::Directory) => format!("Tab  widen it to this whole directory  {alternate}"),
            (PatternScope::Broad, GrantUnit::Directory) => format!("Tab  narrow it back to this one file  {alternate}"),
            (PatternScope::Exact, GrantUnit::Program(program)) => format!("Tab  widen it to every {program} command  {alternate}"),
            (PatternScope::Broad, GrantUnit::Program(_)) => format!("Tab  narrow it back to this one command  {alternate}"),
        });
    }
    lines
}

/// Builds the panel's content: whichever pending approval/prompt is at the
/// front of its queue, wrapped in `Modal.jsx`'s chrome. Returns nothing at
/// all when both queues are empty (the band then collapses to zero height —
/// see `super::draw`).
///
/// `App::pending_front` is the single source of truth for "approvals before
/// prompts", mirroring `App::handle_key`'s own priority — what's shown here
/// must always be exactly what the next keypress resolves.
///
/// The band and the footer are assembled *outside* [`clamp_panel`]'s budget
/// and appended after clamping, so neither can ever be the thing a large
/// diff or a small terminal squeezes out: the footer explains how to use
/// the options list, so it needs the same "never truncated" guarantee the
/// list itself gets.
pub(super) fn panel_lines(app: &App, ctx: Ctx, frame_height: u16) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    let options = app.decision_options();
    if options.is_empty() {
        return Vec::new();
    }
    // The panel's own list, reduced to what a row actually draws — the
    // shape `super::picker` renders too, so both lists are one control.
    let rows: Vec<OptionRow> =
        options.iter().map(|o| OptionRow { label: o.label.clone(), detail: o.detail.clone() }).collect();
    let card = Row::card(pal.bar);
    // The separator above the options list. The design system's permission
    // screen replaced the old accent rule here with "one row of the
    // recessed tone" — a band that sinks below the panel rather than a line
    // drawn across it. Shared by both arms below, since every payload
    // kind's options list gets the same separator ahead of it.
    let options_rule = || vec![card.blank(ctx), band_row(pal.recess, ctx), card.blank(ctx)];
    let queue_note = |queued: usize| -> Vec<Line<'static>> {
        if queued > 1 {
            card.text(&format!("(+{} more pending)", queued - 1), pal.dim, ctx)
        } else {
            Vec::new()
        }
    };
    let budget = max_height(frame_height);

    let (body, badge_text, tail) = match app.pending_front() {
        PendingFront::Approval(pending) => {
            let mut tail = options_rule();
            tail.extend(option_rows(&rows, app.decision_selected, ctx));
            tail.extend(queue_note(app.pending_approvals.len()));
            // Everything the budget has left once the tail is reserved goes
            // to the card, which sizes its own diff box against it — that
            // is what keeps `clamp_panel` below from ever having to cut
            // into the box and leave it unclosed.
            let card_rows = budget.saturating_sub(tail.len());
            (approval_card(&pending.diff, Vec::new(), Some(card_rows), ctx), "edit".to_string(), tail)
        }
        PendingFront::Prompt(pending) => {
            // Present for every Tool prompt (its second line, the Tab
            // toggle, only when the target has an enclosing directory to
            // broaden to); absent for a ContextFile prompt, which persists
            // a path rather than a grant pattern and has no rule to state.
            let mut tail: Vec<Line<'static>> = match app.decision_grant() {
                // Its own padding row above: the rule restates the target
                // the card body just showed, so without a break the two sit
                // as adjacent near-identical rows ("read: ./x.rs" directly
                // over "…adds the rule  read:./x.rs") and read as a stutter
                // rather than as a statement about what happens next.
                Some(grant) => std::iter::once(card.blank(ctx)).chain(grant_lines(&grant).iter().flat_map(|line| card.text(line, pal.dim, ctx))).collect(),
                None => Vec::new(),
            };
            tail.extend(options_rule());
            tail.extend(option_rows(&rows, app.decision_selected, ctx));
            tail.extend(queue_note(app.pending_prompts.len()));
            let view = PromptView::of(&pending.payload);
            (prompt_card(&pending.payload, Vec::new(), ctx), view.badge, tail)
        }
        PendingFront::None => return Vec::new(),
    };

    // `body` is built with an empty tail and the real one appended here, so
    // the two are clamped as one unit — `clamp_panel` protects the whole
    // options list, not just whatever the card happened to leave unclamped.
    let mut clampable = body;
    let tail_len = tail.len();
    clampable.extend(tail);
    let clamped = clamp_panel(clampable, budget, tail_len, ctx);

    let mut lines = vec![band("permission", &badge_text, ctx)];
    lines.extend(clamped);
    lines.push(card.blank(ctx));
    // No rule above the footer. It used to carry the reference's
    // `border-top: 1px solid var(--tui-line)`, but Turn 13 removed that
    // stroke along with every other one: the footer sits on the bottom-bar
    // tone and the step down from the panel's own `bar` is the boundary.
    // The blank row above is `card`'s, so it is still on `bar` — which is
    // what makes the step land exactly where the rule used to.
    // No right-hand provenance note. It used to read "saved to
    // .mjolnir/permissions.yaml" under every prompt, which was true of
    // exactly one of the tiers on offer — "allow once" and "allow for this
    // session" save nothing at all, and "always allow" writes to the global
    // file instead. Where each answer lands is now stated per option, on
    // the option's own row (`DecisionOption::detail`).
    lines.push(Row::card(pal.bar_bottom).split(footer_hint(options.len(), ctx), Vec::new(), ctx));
    lines
}

/// Caps the panel to `max` rows as a last resort, keeping the leading rows
/// (blank padding + sentence) and the caller-supplied `tail` (the options
/// list, any queue-count note) intact and collapsing whatever body content
/// doesn't fit between them into a single marker line.
///
/// After [`panel_lines`] sizes the approval card's diff box against this
/// same budget, the only body that can still overflow is a *prompt* card's
/// wrapped target — an arbitrarily long shell command is ordinary input —
/// and that body contains no bordered box, so cutting it is safe. `tail`
/// (not a fixed constant) is what makes this correct once the options list
/// can be anywhere from 2 rows (Approve/Deny) to 8 (a Tool prompt's four
/// tiers × allow/deny): a fixed guess would either truncate real options
/// away or protect rows that aren't the list.
fn clamp_panel(lines: Vec<Line<'static>>, max: usize, tail: usize, ctx: Ctx) -> Vec<Line<'static>> {
    const HEAD: usize = 2;
    if lines.len() <= max {
        return lines;
    }
    let mut lines = lines;
    let tail_lines = lines.split_off(lines.len().saturating_sub(tail));
    let head_lines: Vec<_> = lines.drain(..HEAD.min(lines.len())).collect();
    // `keep` is found by shrinking until head + kept body + the marker
    // actually fit in `max`, re-measuring the marker on every attempt —
    // regression fix: this used to assume the marker was always exactly one
    // row, but it wraps like any other card row once its ~70-column text is
    // wider than the panel, which is common rather than exotic. The
    // undercounted budget let the *tail* — the one thing that must never be
    // cut — get silently pushed past the panel's real row budget and
    // clipped off the bottom by the outer layout: an ordinary 8-option Tool
    // prompt on a modest terminal lost its last four options entirely, with
    // no on-screen indication anything was missing. `keep == 0` is the
    // floor — even then, no marker is added if there was nothing left to
    // hide, fixing the companion bug where a degenerate head+tail-only
    // panel printed a nonsensical "0 more lines not shown" row it didn't
    // need and couldn't afford.
    let card = Row::card(ctx.pal.bar);
    let mut keep = lines.len();
    loop {
        let hidden = lines.len() - keep;
        let marker: Vec<Line<'static>> = if hidden == 0 {
            Vec::new()
        } else {
            card.text(&format!("⋯ {hidden} more line{} not shown — deciding doesn't require scrolling them ⋯", if hidden == 1 { "" } else { "s" }), ctx.pal.dim, ctx)
        };
        if keep == 0 || head_lines.len() + keep + marker.len() + tail_lines.len() <= max {
            let mut out = head_lines;
            out.extend(lines.into_iter().take(keep));
            out.extend(marker);
            out.extend(tail_lines);
            return out;
        }
        keep -= 1;
    }
}

