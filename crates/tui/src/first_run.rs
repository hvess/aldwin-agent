//! First run — the design system's screen `5d`, and the state behind it.
//!
//! Entering a project with no `.mjolnir/permissions.yaml` asks three
//! questions and then starts: which provider the model runs on, which of its
//! models, and how much runs without asking in this directory. Every answer
//! is written before the session opens, which is why this runs as its own
//! screen with its own terminal loop rather than as a mode inside
//! [`crate::app::App`]: the provider choice decides which LLM client the
//! bootstrap constructs, so it has to be answered before that client exists.
//!
//! The provider is asked *before* the model, and the model list is that
//! provider's own. Turn 13's `5d` ordered it that way: a model id means
//! nothing until you know whose catalogue it comes from, and the provider is
//! the answer that has to be settled before a client can be built at all.
//! The model step used to be absent entirely — first run wrote the
//! provider's catalogue default and left `/model` to change it, which is a
//! decision made silently on the developer's behalf on the one screen whose
//! whole job is to stop that happening.
//!
//! This screen never sees the catalogue itself. [`ProviderChoice`] is the
//! display half of a row — an id, a purpose, and its models — handed in by the caller,
//! because the catalogue lives in `mjolnir-llm` (endpoints, key variables,
//! wire dialects) and this crate does not depend on it.
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

/// One model row: the id that lands in `provider.yaml`, and what picking it
/// does. The same shape as a [`ProviderChoice`] because they are drawn on
/// the same option row — the model step is the provider question narrowed,
/// not a different control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChoice {
    pub id:      String,
    pub purpose: String,
}

impl ModelChoice {
    pub fn new(id: impl Into<String>, purpose: impl Into<String>) -> Self {
        Self { id: id.into(), purpose: purpose.into() }
    }
}

/// One provider row, as the screen needs it: the lowercase id in the
/// option's 16-cell field, what picking it does, and the models it offers —
/// the second question's list, carried on the row it follows from rather
/// than handed in as a separate catalogue this screen would have to join.
///
/// Owned `String`s rather than `&'static str` because the caller composes
/// these from its own catalogue rather than from a literal in this crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderChoice {
    pub id:      String,
    pub purpose: String,
    pub models:  Vec<ModelChoice>,
}

impl ProviderChoice {
    pub fn new(id: impl Into<String>, purpose: impl Into<String>, models: Vec<ModelChoice>) -> Self {
        Self { id: id.into(), purpose: purpose.into(), models }
    }
}

/// Which question is taking arrow keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Provider,
    /// Always follows [`Step::Provider`] and never appears without it: a
    /// model id means nothing until the provider whose catalogue it comes
    /// from is settled, and the list this step shows is the chosen
    /// provider's own.
    Model,
    Access,
}

/// What first run answered. Returned to the bootstrap, which writes it.
///
/// Both fields are optional, and both mean the same thing when absent: the
/// question was not asked, so the caller must leave what is already on disk
/// alone. An `AccessTier` here that the developer never chose would be a
/// permission answered by inertia, which is the one thing this screen exists
/// to prevent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answers {
    /// The chosen provider's id — the caller looks the rest up in the
    /// catalogue it built the [`ProviderChoice`] list from.
    pub provider: Option<String>,
    /// The chosen model's id, from that provider's own list. Present
    /// exactly when `provider` is: the two are one answer, and a provider
    /// written without a model would leave `provider.yaml` half-written.
    pub model:    Option<String>,
    pub access:   Option<AccessTier>,
}

/// The screen's whole state.
///
/// `steps` is built from what is actually unanswered, which is why it is a
/// list rather than a fixed pair. Three cases reach this screen:
///
/// * a true first run — no provider config anywhere, and a directory the
///   harness has never been pointed at — asks `provider`, `model`, then
///   `access`;
/// * entering a fresh project while a provider is already configured asks
///   `access` alone;
/// * losing the provider config in a project that has already declared its
///   access posture asks `provider` and `model`.
///
/// `model` is never a step on its own: it is the provider question
/// narrowed, and its list is the chosen provider's own.
///
/// The `step n/m` counter reads off this list, so a one-question case says
/// "step 1/1" rather than claiming a step that will never come.
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
///
/// The provider list has the same property for a different reason: it opens
/// on the first curated row, and every row on it costs the same — a
/// provider choice grants nothing and is one `/model` away from being
/// changed.
#[derive(Debug, Clone)]
pub struct FirstRun {
    /// Every provider on offer, curated rows first.
    pub providers: Vec<ProviderChoice>,
    /// How many of `providers` show before the `more` row. Clamped to the
    /// list's own length at construction, so a caller cannot promise more
    /// curated rows than it supplied.
    pub curated:   usize,
    /// Set once `more` has been taken: the list becomes the whole catalogue
    /// and the `more` row goes away, having nothing left to reveal.
    pub expanded:  bool,
    pub steps:     Vec<Step>,
    pub index:     usize,
    pub provider:  usize,
    /// Indexes the *selected provider's* model list, so it is reset to 0
    /// whenever the provider selection moves — an index into a list that
    /// changed under it would otherwise pick a model from the wrong
    /// catalogue, or none at all.
    pub model:     usize,
    pub access:    usize,
    /// Set when `⏎` commits the last step, or when the developer quits.
    pub finished:  Option<Option<Answers>>,
}

impl FirstRun {
    /// `providers` is the whole catalogue in display order, curated rows
    /// first; `curated` is how many of them show before `more`.
    ///
    /// The two flags are independent, and only the questions they turn on
    /// are shown. `ask_provider` is false when a provider is already
    /// configured; `ask_access` is false when this directory already has a
    /// `permissions.yaml`. Both directions matter: a screen that asks a
    /// question already answered invites the developer to answer it
    /// differently, and the access answer is written by *adding* grants, so
    /// re-asking it could only ever widen an allow list the developer had
    /// already settled.
    pub fn new(providers: Vec<ProviderChoice>, curated: usize, ask_provider: bool, ask_access: bool) -> Self {
        // An empty catalogue cannot be asked about, whatever the caller
        // said — `commit` would have no id to return.
        let ask_provider = ask_provider && !providers.is_empty();
        let mut steps = Vec::new();
        if ask_provider {
            steps.push(Step::Provider);
            // The model step exists only alongside the provider one, and
            // only when there is a model list to show: a catalogue row with
            // no models would put an empty list on screen and give `commit`
            // nothing to return.
            if providers.iter().any(|p| !p.models.is_empty()) {
                steps.push(Step::Model);
            }
        }
        // A screen with no question on it is not a screen, and `step()`
        // indexes this list. The caller does not open one; if it did, the
        // question to fall back on is the default-deny one.
        if ask_access || steps.is_empty() {
            steps.push(Step::Access);
        }
        let curated = curated.min(providers.len());
        Self { providers, curated, expanded: false, steps, index: 0, provider: 0, model: 0, access: 0, finished: None }
    }

    /// Which question is taking arrow keys.
    pub fn step(&self) -> Step {
        self.steps[self.index.min(self.steps.len() - 1)]
    }

    /// `(n, m)` for the `step n/m` counter — 1-based, over the steps this
    /// run actually has.
    pub fn position(&self, step: Step) -> Option<(usize, usize)> {
        let at = self.steps.iter().position(|s| *s == step)?;
        Some((at + 1, self.steps.len()))
    }

    /// The provider rows currently on screen — the curated prefix until
    /// `more` is taken, the whole catalogue after.
    pub fn visible_providers(&self) -> &[ProviderChoice] {
        if self.expanded {
            &self.providers
        } else {
            &self.providers[..self.curated]
        }
    }

    /// Whether the `more` row is on screen. It goes once taken, and never
    /// appears when the curated prefix is already the whole catalogue.
    pub fn shows_more(&self) -> bool {
        !self.expanded && self.curated < self.providers.len()
    }

    /// The index of the `more` row, when there is one: immediately after
    /// the last visible provider.
    fn more_index(&self) -> Option<usize> {
        self.shows_more().then(|| self.visible_providers().len())
    }

    /// The models of the provider currently selected — the model step's own
    /// list. Empty only for a catalogue row that offers none, which the real
    /// one never does (`mjolnir_llm`'s `every_provider_offers_a_model` pins
    /// it) but which this screen must not index blindly.
    pub fn visible_models(&self) -> &[ModelChoice] {
        self.providers.get(self.provider).map(|p| p.models.as_slice()).unwrap_or_default()
    }

    /// The chosen provider row, if the catalogue has one at the selected
    /// index — what the screen shows for an *answered* provider step.
    pub fn chosen_provider(&self) -> Option<&ProviderChoice> {
        self.providers.get(self.provider)
    }

    pub fn chosen_model(&self) -> Option<&ModelChoice> {
        self.visible_models().get(self.model)
    }

    /// How many rows the current step's list has, `more` included.
    fn len(&self) -> usize {
        match self.step() {
            Step::Provider => self.visible_providers().len() + usize::from(self.shows_more()),
            Step::Model => self.visible_models().len(),
            Step::Access => AccessTier::ORDER.len(),
        }
    }

    /// The selected index of the current step. Always present: every list
    /// opens with a row selected (see [`FirstRun`]'s own doc comment).
    fn selected(&self) -> usize {
        match self.step() {
            Step::Provider => self.provider,
            Step::Model => self.model,
            Step::Access => self.access,
        }
    }

    fn select(&mut self, index: usize) {
        match self.step() {
            // Moving the provider selection invalidates the model index —
            // see the field's own doc comment.
            Step::Provider => {
                if self.provider != index {
                    self.model = 0;
                }
                self.provider = index;
            }
            Step::Model => self.model = index,
            Step::Access => self.access = index,
        }
    }

    /// `↑`/`↓`, clamped at both ends rather than wrapping — a list that
    /// wraps makes it possible to land on the widest access tier by holding
    /// a key down.
    fn step_selection(&mut self, delta: isize) {
        // `max(1)` keeps the clamp's bounds ordered on an empty list, which
        // only a catalogue row with no models can produce.
        let len = self.len().max(1);
        let next = (self.selected() as isize + delta).clamp(0, len as isize - 1) as usize;
        self.select(next);
    }

    /// `⏎` on the provider step's `more` row: the list becomes the whole
    /// catalogue and the selection stays where it is, which is now the first
    /// row `more` revealed rather than `more` itself. Nothing is committed
    /// and no step advances — taking `more` is asking to see the rest of the
    /// question, not answering it.
    fn expand(&mut self) {
        self.expanded = true;
        // The `more` row was the last one; after expanding, that index is
        // the first newly revealed provider. Clamp anyway — a one-entry
        // catalogue with `curated == 1` shows no `more` row at all, so this
        // is unreachable, but `provider` indexes a slice.
        self.provider = self.provider.min(self.providers.len().saturating_sub(1));
    }

    fn commit(&mut self) {
        let provider = self.steps.contains(&Step::Provider).then(|| self.chosen_provider().map(|p| p.id.clone())).flatten();
        // Answered with the provider, never on its own: a model id belongs
        // to the catalogue row above it, so the caller writing one without
        // the other is not a case this screen can produce.
        let model = provider.is_some().then(|| self.chosen_model().map(|m| m.id.clone())).flatten();
        let access = self.steps.contains(&Step::Access).then(|| AccessTier::ORDER[self.access]);
        self.finished = Some(Some(Answers { provider, model, access }));
    }

    /// One key. Returns `true` if the screen is done (see `finished`).
    ///
    /// `⏎` advances to the next step, or commits on the last one. Every step
    /// always has a selection, so `⏎` is never a no-op — see [`FirstRun`]
    /// for why that is worth more than the design system's
    /// unanswered-by-default state. The one row it neither advances nor
    /// commits on is `more`, which expands the list in place.
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
            // `←` reopens the question before this one. The model step is
            // the provider step narrowed, so a developer who picks the
            // wrong provider needs a way back that is not "quit the screen
            // and lose the run" — `Esc` still means exactly that, and is
            // left alone.
            KeyCode::Left => self.index = self.index.saturating_sub(1),
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
                if self.step() == Step::Provider && Some(self.provider) == self.more_index() {
                    self.expand();
                } else if self.index + 1 < self.steps.len() {
                    // The last step commits; any earlier one advances.
                    self.index += 1;
                } else {
                    self.commit();
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
pub async fn run(
    theme: Theme,
    providers: Vec<ProviderChoice>,
    curated: usize,
    ask_provider: bool,
    ask_access: bool,
) -> io::Result<Option<Answers>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let guard = Guard;

    let result = run_loop(&mut terminal, theme, providers, curated, ask_provider, ask_access).await;
    drop(guard);
    restore()?;
    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    theme: Theme,
    providers: Vec<ProviderChoice>,
    curated: usize,
    ask_provider: bool,
    ask_access: bool,
) -> io::Result<Option<Answers>> {
    let mut state = FirstRun::new(providers, curated, ask_provider, ask_access);
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
                return Ok(state.finished.clone().flatten());
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

/// A stand-in catalogue for tests in this crate, shaped like the real one:
/// three curated rows and three more behind `more`. Deliberately not the
/// real ids — a test that hard-codes `mjolnir-llm`'s catalogue would fail
/// every time a provider is added to it.
#[cfg(test)]
pub(crate) fn sample_providers() -> Vec<ProviderChoice> {
    ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"]
        .into_iter()
        .map(|id| {
            ProviderChoice::new(
                id,
                format!("{id} models · {}_API_KEY", id.to_ascii_uppercase()),
                vec![
                    ModelChoice::new(format!("{id}-large"), "balanced; a good default"),
                    ModelChoice::new(format!("{id}-small"), "fast, cheap"),
                ],
            )
        })
        .collect()
}

#[cfg(test)]
pub(crate) const SAMPLE_CURATED: usize = 3;

#[cfg(test)]
impl Default for FirstRun {
    fn default() -> Self {
        Self::new(sample_providers(), SAMPLE_CURATED, true, true)
    }
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
        assert!(!key(&mut state, KeyCode::Enter), "enter on the provider step advances rather than finishing");
        assert_eq!(state.step(), Step::Model);
        assert!(!key(&mut state, KeyCode::Enter), "enter on the model step advances too");
        assert_eq!(state.step(), Step::Access);
        assert!(key(&mut state, KeyCode::Enter), "enter on the last step commits");
        assert_eq!(
            state.finished,
            Some(Some(Answers {
                provider: Some("alpha".into()),
                model:    Some("alpha-large".into()),
                access:   Some(AccessTier::Ask),
            }))
        );
    }

    /// The access-only run: one step, and enter commits it immediately. It
    /// answers no provider question, so it must not claim to have answered
    /// one — a `Some(..)` here would overwrite the provider the developer
    /// already configured.
    #[test]
    fn the_access_only_run_commits_on_the_first_enter_and_names_no_provider() {
        let mut state = FirstRun::new(sample_providers(), SAMPLE_CURATED, false, true);
        assert_eq!(state.step(), Step::Access);
        assert!(key(&mut state, KeyCode::Enter));
        assert_eq!(state.finished, Some(Some(Answers { provider: None, model: None, access: Some(AccessTier::Ask) })));
    }

    /// Clamped, not wrapping — a wrapping list makes it possible to land on
    /// the widest tier by holding a key down.
    #[test]
    fn access_selection_clamps_at_both_ends() {
        let mut state = FirstRun::new(sample_providers(), SAMPLE_CURATED, false, true);
        for _ in 0..10 {
            key(&mut state, KeyCode::Up);
        }
        assert_eq!(state.access, 0);
        for _ in 0..10 {
            key(&mut state, KeyCode::Down);
        }
        assert_eq!(state.access, AccessTier::ORDER.len() - 1);
    }

    /// The provider list is the curated prefix plus one `more` row, and the
    /// selection cannot leave it.
    #[test]
    fn the_provider_list_opens_curated_with_one_more_row() {
        let mut state = FirstRun::default();
        assert_eq!(state.visible_providers().len(), SAMPLE_CURATED);
        assert!(state.shows_more());
        for _ in 0..10 {
            key(&mut state, KeyCode::Down);
        }
        assert_eq!(state.provider, SAMPLE_CURATED, "the last selectable row is `more`, not the last provider");
    }

    /// `more` reveals the rest of the catalogue and takes its own row away
    /// — it has nothing left to show — landing the selection on the first
    /// provider it revealed rather than dumping it back at the top.
    #[test]
    fn more_expands_the_list_in_place_without_committing() {
        let mut state = FirstRun::default();
        for _ in 0..10 {
            key(&mut state, KeyCode::Down);
        }
        assert!(!key(&mut state, KeyCode::Enter), "taking `more` must not finish the screen");
        assert!(state.expanded);
        assert!(!state.shows_more(), "`more` has nothing left to reveal, so it goes");
        assert_eq!(state.visible_providers().len(), sample_providers().len());
        assert_eq!(state.step(), Step::Provider, "and the step has not advanced either");
        assert_eq!(state.providers[state.provider].id, "delta", "the selection lands on the first newly revealed row");
    }

    /// A catalogue with nothing behind `more` shows no `more` row: a row
    /// that reveals nothing is a row that does nothing.
    #[test]
    fn a_fully_curated_catalogue_shows_no_more_row() {
        let providers = sample_providers();
        let state = FirstRun::new(providers.clone(), providers.len(), true, true);
        assert!(!state.shows_more());
        assert_eq!(state.visible_providers().len(), providers.len());
    }

    /// A caller cannot promise more curated rows than it supplied —
    /// `visible_providers` slices on `curated`.
    #[test]
    fn curated_is_clamped_to_the_catalogue_it_was_given() {
        let state = FirstRun::new(sample_providers(), 99, true, true);
        assert_eq!(state.curated, sample_providers().len());
        assert!(!state.shows_more());
    }

    /// The mirror of the access-only run: a directory that has already
    /// answered its access question is not asked it again, and the commit
    /// says so by returning no tier. `add_grant` only ever adds, so an
    /// unasked answer written into an existing `permissions.yaml` could only
    /// widen a list the developer had already settled.
    #[test]
    fn the_provider_only_run_asks_one_question_and_names_no_access_tier() {
        let mut state = FirstRun::new(sample_providers(), SAMPLE_CURATED, true, false);
        assert_eq!(state.steps, vec![Step::Provider, Step::Model], "the model step travels with the provider one");
        assert_eq!(state.position(Step::Provider), Some((1, 2)), "the counter must not claim a step that will never come");
        assert_eq!(state.position(Step::Access), None);
        assert!(!key(&mut state, KeyCode::Enter));
        assert!(key(&mut state, KeyCode::Enter), "the last step commits");
        assert_eq!(
            state.finished,
            Some(Some(Answers { provider: Some("alpha".into()), model: Some("alpha-large".into()), access: None }))
        );
    }

    /// `step()` indexes `steps`, so a screen with no question on it would
    /// panic. The caller never opens one; if it did, the question to fall
    /// back on is the default-deny one.
    #[test]
    fn a_screen_with_nothing_to_ask_falls_back_to_the_default_deny_question() {
        let state = FirstRun::new(sample_providers(), SAMPLE_CURATED, false, false);
        assert_eq!(state.steps, vec![Step::Access]);
        assert_eq!(state.step(), Step::Access);
    }

    /// An empty catalogue cannot be asked about, whatever the caller said,
    /// or the commit would have no id to return.
    #[test]
    fn an_empty_catalogue_skips_the_provider_step_entirely() {
        let mut state = FirstRun::new(Vec::new(), 3, true, true);
        assert_eq!(state.steps, vec![Step::Access]);
        assert!(key(&mut state, KeyCode::Enter));
        assert_eq!(state.finished, Some(Some(Answers { provider: None, model: None, access: Some(AccessTier::Ask) })));
    }

    #[test]
    fn provider_selection_clamps_at_both_ends() {
        let mut state = FirstRun::default();
        for _ in 0..10 {
            key(&mut state, KeyCode::Up);
        }
        assert_eq!(state.provider, 0);
    }

    #[test]
    fn a_digit_picks_that_row_and_out_of_range_digits_are_ignored() {
        let mut state = FirstRun::default();
        key(&mut state, KeyCode::Char('2'));
        assert_eq!(state.provider, 1);
        key(&mut state, KeyCode::Char('9'));
        assert_eq!(state.provider, 1, "a digit past the end of the list must not move the selection");
    }

    #[test]
    fn answering_every_step_yields_every_answer() {
        let mut state = FirstRun::default();
        key(&mut state, KeyCode::Char('2')); // bravo
        key(&mut state, KeyCode::Enter);
        key(&mut state, KeyCode::Char('2')); // bravo-small
        key(&mut state, KeyCode::Enter);
        key(&mut state, KeyCode::Char('3')); // all
        assert!(key(&mut state, KeyCode::Enter));
        assert_eq!(
            state.finished,
            Some(Some(Answers {
                provider: Some("bravo".into()),
                model:    Some("bravo-small".into()),
                access:   Some(AccessTier::All),
            }))
        );
    }

    /// A provider revealed by `more` is answerable like any other — the
    /// expansion is a view change, not a separate mode.
    #[test]
    fn a_provider_from_behind_more_can_be_committed() {
        let mut state = FirstRun::default();
        for _ in 0..10 {
            key(&mut state, KeyCode::Down);
        }
        key(&mut state, KeyCode::Enter); // take `more`
        key(&mut state, KeyCode::Char('6')); // foxtrot
        key(&mut state, KeyCode::Enter); // advance to model
        key(&mut state, KeyCode::Enter); // advance to access
        assert!(key(&mut state, KeyCode::Enter));
        assert_eq!(
            state.finished,
            Some(Some(Answers {
                provider: Some("foxtrot".into()),
                model:    Some("foxtrot-large".into()),
                access:   Some(AccessTier::Ask),
            }))
        );
    }

    /// `←` reopens the question before this one, so a wrong provider is one
    /// keypress away from being fixed rather than a reason to quit the
    /// screen.
    #[test]
    fn left_reopens_the_previous_question_and_stops_at_the_first() {
        let mut state = FirstRun::default();
        key(&mut state, KeyCode::Enter);
        assert_eq!(state.step(), Step::Model);
        key(&mut state, KeyCode::Left);
        assert_eq!(state.step(), Step::Provider);
        key(&mut state, KeyCode::Left);
        assert_eq!(state.step(), Step::Provider, "there is nothing before the first question");
        assert_eq!(state.finished, None, "going back is not quitting");
    }

    /// The model index is a position in *that* provider's list, so moving
    /// off a provider must not carry it onto a different list.
    #[test]
    fn moving_the_provider_selection_resets_the_model_selection() {
        let mut state = FirstRun::default();
        key(&mut state, KeyCode::Enter); // to the model step
        key(&mut state, KeyCode::Down); // alpha-small
        assert_eq!(state.model, 1);
        key(&mut state, KeyCode::Left); // back to the providers
        key(&mut state, KeyCode::Down); // bravo
        assert_eq!(state.model, 0, "a new provider's list opens at its top");
    }

    /// The model step travels with the provider one and never appears
    /// without it — a model id means nothing until the catalogue it comes
    /// from is settled.
    #[test]
    fn the_model_step_is_asked_only_when_the_provider_is() {
        let asked = FirstRun::default();
        assert_eq!(asked.steps, vec![Step::Provider, Step::Model, Step::Access]);
        let access_only = FirstRun::new(sample_providers(), SAMPLE_CURATED, false, true);
        assert_eq!(access_only.steps, vec![Step::Access]);
    }

    /// A catalogue whose rows offer no models has no second question to
    /// ask, and must not put an empty list on screen.
    #[test]
    fn a_catalogue_with_no_models_asks_no_model_step() {
        let providers = vec![ProviderChoice::new("solo", "one endpoint · SOLO_API_KEY", Vec::new())];
        let mut state = FirstRun::new(providers, 1, true, false);
        assert_eq!(state.steps, vec![Step::Provider]);
        assert!(key(&mut state, KeyCode::Enter));
        assert_eq!(
            state.finished,
            Some(Some(Answers { provider: Some("solo".into()), model: None, access: None })),
            "the caller then writes that provider's own default"
        );
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
