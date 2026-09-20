//! The pickers' panel — bare `/model` and bare `/resume`, drawn in the
//! bottom band.
//!
//! It is the permission panel's own shape, deliberately: a risen title row,
//! a row of prose, a recessed rule, the numbered option rows, and the key
//! hints. That panel is the system's one in-session modal, so a second
//! control invented for this would read as a different application. The
//! only differences are the ones the question forces — the title names the
//! stage, and the rows come from the catalogue rather than from a pending
//! decision.
//!
//! What it does *not* borrow is first run's indented option row. That row
//! sits on the body column because it lives in a full-frame form; a row
//! inside this panel is flush to the frame's left edge, like every other
//! row the panel draws.

use ratatui::text::Line;

use super::decision::{band, key_hints, option_rows, OptionRow};
use super::grid::Ctx;
use super::row::Row;
use crate::app::App;
use crate::picker::Stage;

/// The rows between the title and the option list, shared by both pickers.
///
/// **Title row, blank, sentence, blank, options** — the permission panel's
/// own sequence (`decision::panel_lines`), which is the whole point: these
/// are one control wearing two questions.
///
/// Note what is *not* here: a `recess` band above the options. `HANDOFF.md`'s
/// prose asks for "one row of the recessed tone, blank row" and `5a`'s own
/// rendered frame has a single blank `<div>` and no `--t-recess` anywhere —
/// settled in `crates/review/baseline.json` as
/// `5a-recess-in-the-prose-but-not-in-the-frame`, where the app follows the
/// frame. The permission panel always did; these two did not, because they
/// were written from the prose. A stage 5 judge caught it on the session
/// list and it was true of the model picker too.
///
/// What each stage is for, in one row of prose above its list.
const PROVIDER_PROSE: &str = "Where the model runs.";
const MODEL_PROSE: &str = "Which model this provider answers with.";

/// How a row says it is the one the session is already running on. Not a
/// glyph: the vocabulary is closed, and this is a fact about the row rather
/// than a mark on it.
const CURRENT_NOTE: &str = " · current";

/// What the session list is for, in one row of prose above it.
const RESUME_PROSE: &str = "A past conversation, picked up where it stopped.";

/// The panel's lines, or nothing at all when no picker is open — the band
/// then collapses to zero height, the same contract `decision::panel_lines`
/// has.
///
/// Both pickers draw through here rather than each owning a panel: they ask
/// different questions but they are the same control, and a second
/// implementation of the shape would be free to drift from it. They are
/// mutually exclusive — a submission is what opens either, and no submission
/// can happen while one holds the band.
pub(super) fn panel_lines(app: &App, ctx: Ctx) -> Vec<Line<'static>> {
    if app.resume.is_some() {
        return resume_lines(app, ctx);
    }
    let Some(picker) = app.picker.as_ref() else { return Vec::new() };
    let pal = ctx.pal;
    let card = Row::card(pal.bar);

    let (title, prose) = match picker.stage {
        Stage::Provider => ("provider".to_string(), PROVIDER_PROSE),
        // The provider is named in the badge rather than the prose: the
        // second list is a list of *that* catalogue's models, and the
        // developer needs to see which one they are looking at.
        Stage::Model => ("model".to_string(), MODEL_PROSE),
    };
    let badge = match picker.stage {
        Stage::Provider => "model".to_string(),
        Stage::Model => picker.provider_id().to_string(),
    };

    let rows: Vec<OptionRow> = picker
        .rows()
        .into_iter()
        .map(|row| OptionRow {
            label:   row.label,
            detail:  if row.current { format!("{}{CURRENT_NOTE}", row.detail) } else { row.detail },
            // `5c`'s pair shape quotes no grant pattern — see `OptionRow`.
            pattern: None,
        })
        .collect();

    let mut lines = vec![band(&title, &badge, ctx)];
    lines.push(card.blank(ctx));
    lines.extend(card.text(prose, pal.body, ctx));
    lines.push(card.blank(ctx));
    lines.extend(option_rows(&rows, picker.selected(), ctx));
    lines.push(card.blank(ctx));
    // `esc` is named because it does something different on each stage —
    // it reopens the provider list from the model one, and closes the
    // picker from the provider one — and a modal that takes the whole
    // bottom band has to say how to leave it.
    let back = match picker.stage {
        Stage::Provider => "to close",
        Stage::Model => "to go back",
    };
    let hints = key_hints(
        &[("↑↓", "to move"), (&format!("1-{}", rows.len()), "to pick"), ("⏎", "to confirm"), ("esc", back)],
        ctx,
    );
    lines.push(Row::card(pal.bar_bottom).split(hints, Vec::new(), ctx));
    lines
}

/// The session list — one stage, so `esc` has only the one meaning and the
/// hints say so.
fn resume_lines(app: &App, ctx: Ctx) -> Vec<Line<'static>> {
    let Some(picker) = app.resume.as_ref() else { return Vec::new() };
    let pal = ctx.pal;
    let card = Row::card(pal.bar);

    let rows: Vec<OptionRow> = picker
        .rows()
        .into_iter()
        .map(|row| OptionRow { label: row.label, detail: row.detail, pattern: None })
        .collect();

    let mut lines = vec![band("resume", "session", ctx)];
    lines.push(card.blank(ctx));
    lines.extend(card.text(RESUME_PROSE, pal.body, ctx));
    lines.push(card.blank(ctx));
    lines.extend(option_rows(&rows, picker.selected(), ctx));
    lines.push(card.blank(ctx));
    let hints = key_hints(
        &[("↑↓", "to move"), (&format!("1-{}", rows.len()), "to pick"), ("⏎", "to confirm"), ("esc", "to close")],
        ctx,
    );
    lines.push(Row::card(pal.bar_bottom).split(hints, Vec::new(), ctx));
    lines
}
