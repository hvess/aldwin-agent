//! First run — the design system's screen `5d`, and the state behind it.
//!
//! Entering a project with no `.mjolnir/permissions.yaml` asks two questions
//! and then starts: which model to default to, and how much runs without
//! asking in this directory. Both answers are written before the session
//! opens, which is why this runs as its own screen with its own terminal
//! loop rather than as a mode inside [`crate::app::App`]: the model choice
//! decides which LLM client the bootstrap constructs, so it has to be
//! answered before that client exists.
//!
//! The screen is also the one place the brand is set as a mark. Per the
//! design system's Brand mark section the wordmark is "one row, never a
//! block" — a multi-row block-character wordmark was built and cut, because
//! "at 15px it dominated a frame whose whole argument is that nothing
//! shouts" — and it "appears on first run and nowhere else".
//!
//! Editing is deliberately absent from the access scale. Per ADR 0001 an
//! edit is never grantable at any tier: `Engine::check_tool` refuses
//! `edit_class` before consulting any list, so no answer here can turn that
//! off, and none of the tiers below claims to.

use std::io;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{Event as CtEvent, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::crossterm::{execute, ExecutableCommand};
use ratatui::Terminal;

use futures::StreamExt;

use crate::palette::Theme;
use crate::ui;

/// How much runs without asking in this directory.
///
/// Three points, not the design system's four — see ADR 0001. The fourth
/// (`write`) described a state that cannot exist once editing is de-scoped,
/// and a drafted replacement (`run`) would have written exactly the same
/// grants as `read`. Each point below writes a different set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessTier {
    Ask,
    Read,
    All,
}

impl AccessTier {
    /// In order, widening — the order the rows are shown in.
    pub const ORDER: [AccessTier; 3] = [AccessTier::Ask, AccessTier::Read, AccessTier::All];

    /// The lowercase name in the option's 16-cell field.
    pub fn label(self) -> &'static str {
        match self {
            AccessTier::Ask => "ask",
            AccessTier::Read => "read",
            AccessTier::All => "all",
        }
    }

    /// What picking it does. Every option row in the system carries one of
    /// these; it says what the choice *does*, not what it is called.
    ///
    /// Each names edits explicitly rather than leaving them implied. The
    /// design's own copy for its top tier was "everything runs, nothing
    /// asks", which would be false here in the one place a developer most
    /// needs it to be true.
    pub fn purpose(self) -> &'static str {
        match self {
            AccessTier::Ask => "every tool asks, every time",
            AccessTier::Read => "reads run; commands and edits ask",
            AccessTier::All => "reads and any command run; edits ask",
        }
    }

    /// The `kind:pattern` allow entries this tier writes. Deliberately no
    /// `edit:` entry at any tier — the engine would ignore one, and writing
    /// a rule that does nothing would misrepresent the harness in its own
    /// config file.
    pub fn grants(self) -> Vec<String> {
        match self {
            AccessTier::Ask => Vec::new(),
            AccessTier::Read => vec!["read:**".into(), "explain:**".into()],
            AccessTier::All => vec!["read:**".into(), "explain:**".into(), "shell:*".into()],
        }
    }
}

/// One model on offer. `id` is what lands in `provider.yaml`; `label` is the
/// short name shown in the option's 16-cell field, since a full model id
/// does not fit it and the design's own mock uses short names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelChoice {
    pub id:      &'static str,
    pub label:   &'static str,
    pub purpose: &'static str,
}

/// The models first run offers. Not every model the provider has — three
/// named points on a speed/depth scale, which is what the question is
/// actually asking. `provider.yaml` takes any id afterwards.
pub const MODELS: [ModelChoice; 3] = [
    ModelChoice { id: "claude-sonnet-5", label: "sonnet-5", purpose: "balanced; a good default" },
    ModelChoice { id: "claude-opus-5", label: "opus-5", purpose: "slower, deeper" },
    ModelChoice { id: "claude-haiku-4-5-20251001", label: "haiku-4.5", purpose: "fast, cheap" },
];

/// Which question is taking arrow keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Model,
    Access,
}

/// What first run answered. Returned to the bootstrap, which writes both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Answers {
    pub model:  &'static str,
    pub access: AccessTier,
}

/// The screen's whole state.
///
/// `steps` is built from what is actually unanswered, which is why it is a
/// list rather than a fixed pair. Two cases reach this screen:
///
/// * a true first run — no provider config anywhere — asks `model` then
///   `access`;
/// * entering a project that has no `.mjolnir/permissions.yaml` while a
///   model is already configured asks `access` alone.
///
/// The `step n of m` counter reads off this list, so the one-question case
/// says "step 1 of 1" rather than claiming a step that will never come.
///
/// `access` starts on `ask`, the most restrictive tier.
///
/// This is a deliberate departure from the design system, which says
/// "nothing is preselected on `access`. Every row shows an idle `▌`, which
/// is how the frame says a decision is still open." That reads well and
/// behaved badly: with nothing selected, `⏎` had to refuse to commit, so a
/// developer pressing it saw a screen that simply did not respond — most
/// visibly on the access-only run, where `access` is the *first* step and
/// the very first key press appeared to do nothing.
///
/// Preselecting is safe here only because of *which* row is preselected.
/// `ask` grants nothing at all, so the default answer is the default-deny
/// one and an accidental `⏎` widens no permission. A preselected `read` or
/// `all` would be the thing the design system is guarding against — a
/// security question answered by inertia — and must not be introduced.
#[derive(Debug, Clone)]
pub struct FirstRun {
    pub steps:    Vec<Step>,
    pub index:    usize,
    pub model:    usize,
    pub access:   usize,
    /// Set when `⏎` commits the last step, or when the developer quits.
    pub finished: Option<Option<Answers>>,
}

impl Default for FirstRun {
    fn default() -> Self {
        Self::new(true)
    }
}

impl FirstRun {
    /// `ask_model` is false when a model is already configured and only the
    /// directory's access posture is unanswered.
    pub fn new(ask_model: bool) -> Self {
        let steps = if ask_model { vec![Step::Model, Step::Access] } else { vec![Step::Access] };
        Self { steps, index: 0, model: 0, access: 0, finished: None }
    }

    /// Which question is taking arrow keys.
    pub fn step(&self) -> Step {
        self.steps[self.index.min(self.steps.len() - 1)]
    }

    /// `(n, m)` for the `step n of m` counter — 1-based, over the steps this
    /// run actually has.
    pub fn position(&self, step: Step) -> Option<(usize, usize)> {
        let at = self.steps.iter().position(|s| *s == step)?;
        Some((at + 1, self.steps.len()))
    }

    /// How many rows the current step's list has.
    fn len(&self) -> usize {
        match self.step() {
            Step::Model => MODELS.len(),
            Step::Access => AccessTier::ORDER.len(),
        }
    }

    /// The selected index of the current step. Always present: every list
    /// opens with a row selected (see [`FirstRun::access`]).
    fn selected(&self) -> usize {
        match self.step() {
            Step::Model => self.model,
            Step::Access => self.access,
        }
    }

    fn select(&mut self, index: usize) {
        match self.step() {
            Step::Model => self.model = index,
            Step::Access => self.access = index,
        }
    }

    /// `↑`/`↓`, clamped at both ends rather than wrapping — a list that
    /// wraps makes it possible to land on the widest access tier by holding
    /// a key down.
    fn step_selection(&mut self, delta: isize) {
        let len = self.len();
        let next = (self.selected() as isize + delta).clamp(0, len as isize - 1) as usize;
        self.select(next);
    }

    /// One key. Returns `true` if the screen is done (see `finished`).
    ///
    /// `⏎` advances to the next step, or commits on the last one. Every step
    /// always has a selection, so `⏎` is never a no-op — see
    /// [`FirstRun::access`] for why that is worth more than the design
    /// system's unanswered-by-default state.
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        if matches!(code, KeyCode::Char('c') | KeyCode::Char('d')) && modifiers.contains(KeyModifiers::CONTROL) {
            self.finished = Some(None);
            return true;
        }
        match code {
            KeyCode::Esc => {
                self.finished = Some(None);
                return true;
            }
            KeyCode::Up => self.step_selection(-1),
            KeyCode::Down => self.step_selection(1),
            // A direct-pick accelerator, the same one the permission panel
            // uses. Only within the current step's own list.
            KeyCode::Char(c) if c.is_ascii_digit() => {
                if let Some(index) = c.to_digit(10).and_then(|d| (d as usize).checked_sub(1)) {
                    if index < self.len() {
                        self.select(index);
                    }
                }
            }
            KeyCode::Enter => {
                // The last step commits; any earlier one advances.
                if self.index + 1 < self.steps.len() {
                    self.index += 1;
                } else {
                    self.finished =
                        Some(Some(Answers { model: MODELS[self.model].id, access: AccessTier::ORDER[self.access] }));
                    return true;
                }
            }
            _ => {}
        }
        false
    }
}

/// Runs the first-run screen to completion on its own terminal.
///
/// `Ok(None)` means the developer quit without answering — the caller must
/// treat that as "do not start a session", not as a set of defaults.
///
/// Mirrors [`crate::run::run`]'s terminal handling, including restoring the
/// terminal on an error or a panic unwinding out of the loop.
pub async fn run(theme: Theme, ask_model: bool) -> io::Result<Option<Answers>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let guard = Guard;

    let result = run_loop(&mut terminal, theme, ask_model).await;
    drop(guard);
    restore()?;
    result
}

async fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, theme: Theme, ask_model: bool) -> io::Result<Option<Answers>> {
    let mut state = FirstRun::new(ask_model);
    let mut events = EventStream::new();
    let pal = theme.palette();

    loop {
        terminal.draw(|frame| ui::first_run::draw(frame, &state, pal))?;
        let Some(event) = events.next().await else {
            // stdin closed — the developer cannot answer, so nothing is
            // written and nothing is assumed.
            return Ok(None);
        };
        if let CtEvent::Key(key) = event? {
            // Press only: a key that repeats or releases must not advance a
            // step twice, the same discipline the session loop uses.
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if state.handle_key(key.code, key.modifiers) {
                return Ok(state.finished.flatten());
            }
        }
    }
}

/// Restores the terminal on the way out, however that happens.
struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = restore();
    }
}

fn restore() -> io::Result<()> {
    let _ = disable_raw_mode();
    let mut stdout = io::stdout();
    let _ = execute!(stdout, LeaveAlternateScreen);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(state: &mut FirstRun, code: KeyCode) -> bool {
        state.handle_key(code, KeyModifiers::NONE)
    }

    /// `ask` is preselected, and it must be `ask` specifically: it is the
    /// tier that grants nothing, so the default answer is the default-deny
    /// one and an accidental `⏎` can never widen a permission.
    #[test]
    fn access_starts_on_ask_the_tier_that_grants_nothing() {
        let state = FirstRun::default();
        assert_eq!(AccessTier::ORDER[state.access], AccessTier::Ask);
        assert!(AccessTier::ORDER[state.access].grants().is_empty(), "the preselected tier must grant nothing");
    }

    /// Enter is never a no-op. Before `ask` was preselected, enter on an
    /// unanswered access step did nothing at all, so on the access-only run
    /// — where access is the *first* step — the very first key press
    /// appeared to leave the screen frozen.
    #[test]
    fn enter_always_advances_or_commits() {
        let mut state = FirstRun::default();
        assert!(!key(&mut state, KeyCode::Enter), "enter on the model step advances rather than finishing");
        assert_eq!(state.step(), Step::Access);
        assert!(key(&mut state, KeyCode::Enter), "enter on the last step commits");
        assert_eq!(state.finished, Some(Some(Answers { model: MODELS[0].id, access: AccessTier::Ask })));
    }

    /// The access-only run: one step, and enter commits it immediately.
    #[test]
    fn the_access_only_run_commits_on_the_first_enter() {
        let mut state = FirstRun::new(false);
        assert_eq!(state.step(), Step::Access);
        assert!(key(&mut state, KeyCode::Enter));
        assert_eq!(state.finished, Some(Some(Answers { model: MODELS[0].id, access: AccessTier::Ask })));
    }

    /// Clamped, not wrapping — a wrapping list makes it possible to land on
    /// the widest tier by holding a key down.
    #[test]
    fn access_selection_clamps_at_both_ends() {
        let mut state = FirstRun::new(false);
        for _ in 0..10 {
            key(&mut state, KeyCode::Up);
        }
        assert_eq!(state.access, 0);
        for _ in 0..10 {
            key(&mut state, KeyCode::Down);
        }
        assert_eq!(state.access, AccessTier::ORDER.len() - 1);
    }

    #[test]
    fn selection_clamps_at_both_ends() {
        let mut state = FirstRun::default();
        for _ in 0..10 {
            key(&mut state, KeyCode::Up);
        }
        assert_eq!(state.model, 0);
        for _ in 0..10 {
            key(&mut state, KeyCode::Down);
        }
        assert_eq!(state.model, MODELS.len() - 1);
    }

    #[test]
    fn a_digit_picks_that_row_and_out_of_range_digits_are_ignored() {
        let mut state = FirstRun::default();
        key(&mut state, KeyCode::Char('2'));
        assert_eq!(state.model, 1);
        key(&mut state, KeyCode::Char('9'));
        assert_eq!(state.model, 1, "a digit past the end of the list must not move the selection");
    }

    #[test]
    fn answering_both_steps_yields_both_answers() {
        let mut state = FirstRun::default();
        key(&mut state, KeyCode::Char('2')); // opus
        key(&mut state, KeyCode::Enter);
        key(&mut state, KeyCode::Char('3')); // all
        assert!(key(&mut state, KeyCode::Enter));
        assert_eq!(state.finished, Some(Some(Answers { model: "claude-opus-5", access: AccessTier::All })));
    }

    #[test]
    fn esc_quits_without_answering() {
        let mut state = FirstRun::default();
        assert!(key(&mut state, KeyCode::Esc));
        assert_eq!(state.finished, Some(None), "quitting must yield no answers at all, not defaults");
    }

    /// Editing is not grantable at any tier (ADR 0001), so no tier may
    /// write an `edit:` rule — one would be silently ignored by the engine
    /// and would misstate the harness in its own config file.
    #[test]
    fn no_tier_grants_edit() {
        for tier in AccessTier::ORDER {
            assert!(!tier.grants().iter().any(|g| g.starts_with("edit:")), "{} must not grant edit", tier.label());
        }
    }

    /// Each tier writes a strictly different set — the reason there are
    /// three points and not the design's four.
    #[test]
    fn every_tier_writes_a_distinct_grant_set() {
        let sets: Vec<Vec<String>> = AccessTier::ORDER.iter().map(|t| t.grants()).collect();
        assert_eq!(sets[0].len(), 0, "ask writes nothing at all");
        for pair in sets.windows(2) {
            assert_ne!(pair[0], pair[1], "a named tier that writes what its neighbour writes is not a choice");
            assert!(pair[0].len() < pair[1].len(), "the scale must widen in order");
        }
    }
}
