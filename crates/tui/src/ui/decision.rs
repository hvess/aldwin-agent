//! The decision panel: the one interactive surface for a pending Edit
//! approval or permission prompt, plus the resolved cards those leave
//! behind in the log.
//!
//! Structure, top to bottom, matching the design system's Permission
//! screen: a title band, the card body (a sentence, the target, and either
//! a bordered diff or a command block), the grant the answer would save,
//! a rule, the numbered options, and a key-hint footer.

use mjolnir_permissions::PromptPayload;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use super::diff;
use super::grid::{Ctx, MARGIN_X};
use super::row::{rule_row, Row};
use crate::app::{App, DecisionOption, GrantSummary, PatternScope, PendingFront};

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
    const RESERVED_FOR_REST_OF_UI: u16 = 3 /* top bar */ + 1 /* top bar rule */ + 1 /* one row of log */ + 1 /* bottom bar edge */;
    // [`panel_lines`] adds the band's row and the footer's 3 rows *outside*
    // the budget this bounds (see there for why) — reserved here too, so
    // the combined total still fits the same overall budget, not just the
    // clamped body alone.
    const PANEL_CHROME: u16 = 1 /* band */ + 3 /* footer padding + rule + hint */;
    (frame_height.saturating_sub(RESERVED_FOR_REST_OF_UI + PANEL_CHROME) as usize).max(6)
}

/// Wrapped-row count of `lines` at `width` — the same
/// `Paragraph::line_count` technique `transcript::row_count` uses, so the
/// height `super::draw` reserves and what [`draw_panel`] actually renders
/// can never disagree.
pub(super) fn row_count(lines: &[Line<'static>], width: u16) -> usize {
    Paragraph::new(Text::from(lines.to_vec())).wrap(Wrap { trim: false }).line_count(width)
}

pub(super) fn draw_panel(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    if area.height == 0 {
        return;
    }
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

/// The Edit approval card: a sentence, the path, and the diff as a real
/// bordered box (`InlineDiff.jsx`: `border: 1px solid var(--tui-line)`) on
/// `diff_box` — one step off the card's `bar` field, so it reads as "a
/// quoted block inside this card," the same nesting the command block uses
/// for a different payload kind.
///
/// Shared by two very different call sites: the live panel (`resolution:
/// None`, `tail` carries the numbered options list and footer, built and
/// owned by the caller) and a resolved entry's permanent record inline in
/// the log (`resolution: Some(_)`, `tail` unused since the "resolved: …"
/// line takes its place).
///
/// `card_rows` caps the whole card — [`panel_lines`] passes what's left of
/// the panel's budget once the tail is known, and the diff box is sized
/// against whatever the card's own (wrappable, so measured rather than
/// assumed) head leaves of that. The card therefore always fits its budget,
/// which is what keeps [`clamp_panel`] from ever cutting into the box and
/// leaving it unclosed. `None` (the resolved-in-log path, where the log
/// scrolls) shows the whole diff.
pub(super) fn approval_card(diff_text: &str, resolution: Option<bool>, tail: Vec<Line<'static>>, ctx: Ctx) -> Vec<Line<'static>> {
    approval_card_bounded(diff_text, resolution, tail, None, ctx)
}

fn approval_card_bounded(diff_text: &str, resolution: Option<bool>, tail: Vec<Line<'static>>, card_rows: Option<usize>, ctx: Ctx) -> Vec<Line<'static>> {
    /// Two borders, one diff row and one elision marker — below this a box
    /// can't say anything a plain note wouldn't say better.
    const MIN_BOX_ROWS: usize = 4;

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
        lines.extend(card.text(&path, pal.label, ctx));
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
        lines.extend(diff::boxed(&body, budget, Row::boxed(pal.diff_box).inset(MARGIN_X, pal.bar), ctx));
    }

    match resolution {
        Some(approved) => {
            let (word, fg) = if approved { ("approved", pal.add) } else { ("denied", pal.del) };
            lines.push(card.blank(ctx));
            lines.extend(card.text(&format!("resolved: {word}"), fg, ctx));
            lines.push(card.blank(ctx));
        }
        None => lines.extend(tail),
    }
    lines
}

/// The permission-prompt card — same two call sites and the same `tail`
/// contract as [`approval_card`].
pub(super) fn prompt_card(payload: &PromptPayload, resolution: Option<&str>, tail: Vec<Line<'static>>, ctx: Ctx) -> Vec<Line<'static>> {
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
    match resolution {
        Some(r) => {
            lines.extend(card.text(&format!("resolved: {r}"), pal.accent_text, ctx));
            lines.push(card.blank(ctx));
        }
        None => lines.extend(tail),
    }
    lines
}

/// `CommandBlock.jsx`: a `ground`-coloured field (distinct from the card's
/// own `bar` surface, so it reads as an inset quoted block — the same
/// nesting the diff box uses for a different payload kind) with the command
/// prefixed by an accent `$`.
fn command_block(command: &str, ctx: Ctx) -> Vec<Line<'static>> {
    let pal = ctx.pal;
    let row = Row::card(pal.ground);
    let spans = vec![Span::styled("$ ", Style::default().fg(pal.speaker_you)), Span::styled(command.to_string(), Style::default().fg(pal.text))];
    let mut lines = vec![row.blank(ctx)];
    lines.extend(row.build(spans, ctx));
    lines.push(row.blank(ctx));
    lines
}

/// `Modal.jsx`'s title row: a field of `band` carrying the plain-lowercase
/// kind (`permission`) in `accent_text`, no glyph (per the design system's
/// revision log: a `▌` pip "indicated nothing" here), and the payload's own
/// kind right-aligned. No rule of its own: the panel's single `border-top`
/// is drawn by `super::draw`, on the row the bottom bar's own edge would
/// otherwise occupy.
fn band(title: &str, badge: &str, ctx: Ctx) -> Line<'static> {
    Row::card(ctx.pal.band).split(vec![Span::styled(title.to_string(), Style::default().fg(ctx.pal.accent_text))], badge, ctx)
}

/// "↑↓ to move   1-N to pick   ⏎ to confirm" — `KeyHints.jsx`'s
/// key-coloured/verb-muted pair convention. Replaces the per-row shortcut
/// column the panel used to need, per the design system's revision log on
/// the permission screen: "the keys that were on the rows moved into the
/// footer."
fn footer_hint(option_count: usize, ctx: Ctx) -> Vec<Span<'static>> {
    let key = Style::default().fg(ctx.pal.mark);
    let verb = Style::default().fg(ctx.pal.quiet);
    vec![
        Span::styled("↑↓", key),
        Span::styled(" to move      ", verb),
        Span::styled(format!("1-{option_count}"), key),
        Span::styled(" to pick      ", verb),
        Span::styled("⏎", key),
        Span::styled(" to confirm", verb),
    ]
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
fn option_rows(options: &[DecisionOption], selected: usize, ctx: Ctx) -> Vec<Line<'static>> {
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
        lines.push(match grant.scope {
            PatternScope::Exact => format!("Tab  widen it to this whole directory  {alternate}"),
            PatternScope::Directory => format!("Tab  narrow it back to this one file  {alternate}"),
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
    let card = Row::card(pal.bar);
    // The flat `rule` above the options list — `readme.md`: freestanding
    // rules are "one step more muted than the structural borders they sit
    // beside." Shared by both arms below, since every payload kind's
    // options list gets the same rule ahead of it.
    let options_rule = || vec![card.blank(ctx), rule_row(pal.rule, pal.bar, ctx), card.blank(ctx)];
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
            tail.extend(option_rows(&options, app.decision_selected, ctx));
            tail.extend(queue_note(app.pending_approvals.len()));
            // Everything the budget has left once the tail is reserved goes
            // to the card, which sizes its own diff box against it — that
            // is what keeps `clamp_panel` below from ever having to cut
            // into the box and leave it unclosed.
            let card_rows = budget.saturating_sub(tail.len());
            (approval_card_bounded(&pending.diff, None, Vec::new(), Some(card_rows), ctx), "edit".to_string(), tail)
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
            tail.extend(option_rows(&options, app.decision_selected, ctx));
            tail.extend(queue_note(app.pending_prompts.len()));
            let view = PromptView::of(&pending.payload);
            (prompt_card(&pending.payload, None, Vec::new(), ctx), view.badge, tail)
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
    // `line`, not the more muted `rule` — matches the reference's real
    // `border-top: 1px solid var(--tui-line)` on this footer row (the rule
    // above the *options* list is the one place `rule` is correct).
    lines.push(rule_row(pal.line, pal.bar_bottom, ctx));
    // No right-hand provenance note. It used to read "saved to
    // .mjolnir/permissions.yaml" under every prompt, which was true of
    // exactly one of the tiers on offer — "allow once" and "allow for this
    // session" save nothing at all, and "always allow" writes to the global
    // file instead. Where each answer lands is now stated per option, on
    // the option's own row (`DecisionOption::detail`).
    lines.push(Row::card(pal.bar_bottom).split(footer_hint(options.len(), ctx), "", ctx));
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

/// Truncates to `max` characters with a trailing `…` — the design system's
/// own elision glyph.
fn elide(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}
