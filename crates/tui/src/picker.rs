//! The in-session model picker — what bare `/model` opens.
//!
//! Two stages over one list control: the providers the catalogue knows, then
//! the models of whichever one was taken. The same order, and the same
//! reason, as [`crate::first_run`]'s two steps — a model id means nothing
//! until the provider whose catalogue it comes from is settled.
//!
//! **It answers by typing the command, not by writing config.** Committing
//! composes `/model <provider>/<model>` and submits it exactly as if the
//! developer had typed it, so mjolnir-cli's interceptor stays the one place
//! that decides which scope to write and what to report. This screen owns
//! how the question is *asked*; it owns nothing about what the answer does.
//! Per mjolnir-cli.md the CLI owns the dispatch table, and a picker that
//! wrote `provider.yaml` itself would be a second implementation of `/model`
//! sitting in the frontend, free to disagree with the first.
//!
//! The catalogue reaches this crate the same way first run's does: as
//! display halves handed in by the caller ([`ProviderChoice`]), never as
//! endpoints or key variables, which belong to mjolnir-llm.

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use crate::first_run::ProviderChoice;

/// Which of the two lists is taking keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Provider,
    Model,
}

/// What one keypress did to the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOutcome {
    /// Still open — redraw and wait for the next key.
    Stay,
    /// Dismissed without choosing. Nothing is submitted and nothing is
    /// written: the session keeps the model it started on.
    Close,
    /// Both halves picked. The caller submits `/model provider/model`.
    Chosen { provider: String, model: String },
}

/// One row of whichever list is on screen: the name in the option row's
/// name field, and what picking it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerRow {
    pub label:  String,
    pub detail: String,
    /// The row the session is already running on — marked so it stays
    /// findable after the cursor has moved off it.
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPicker {
    pub providers: Vec<ProviderChoice>,
    pub stage:     Stage,
    pub provider:  usize,
    pub model:     usize,
    /// Where the session stands — `(provider id, model id)`. The provider
    /// half is absent when `provider.yaml` points at an endpoint the
    /// catalogue has never seen, which is a real configuration and not an
    /// error: the developer's own host has no row to preselect.
    pub current:   (Option<String>, String),
}

impl ModelPicker {
    /// Opens on the row the session is already running on, so the first
    /// thing the list says is where the developer stands. `None` when there
    /// is no catalogue to show — the command then falls through to
    /// mjolnir-cli's own `/model`, which reports rather than picks.
    pub fn open(providers: Vec<ProviderChoice>, current_provider: Option<&str>, current_model: &str) -> Option<Self> {
        if providers.is_empty() {
            return None;
        }
        let provider = current_provider.and_then(|id| providers.iter().position(|p| p.id == id)).unwrap_or(0);
        let model = providers[provider].models.iter().position(|m| m.id == current_model).unwrap_or(0);
        Some(Self {
            providers,
            stage: Stage::Provider,
            provider,
            model,
            current: (current_provider.map(String::from), current_model.to_string()),
        })
    }

    /// The rows of the stage on screen.
    pub fn rows(&self) -> Vec<PickerRow> {
        let (current_provider, current_model) = (&self.current.0, &self.current.1);
        match self.stage {
            Stage::Provider => self
                .providers
                .iter()
                .map(|p| PickerRow {
                    label:   p.id.clone(),
                    detail:  p.purpose.clone(),
                    current: current_provider.as_deref() == Some(p.id.as_str()),
                })
                .collect(),
            Stage::Model => self
                .models()
                .iter()
                .map(|m| PickerRow {
                    label:   m.id.clone(),
                    detail:  m.purpose.clone(),
                    // Only on the provider actually in use: the same model
                    // id under a different host is a different thing to run.
                    current: current_provider.as_deref() == Some(self.providers[self.provider].id.as_str())
                        && *current_model == m.id,
                })
                .collect(),
        }
    }

    fn models(&self) -> &[crate::first_run::ModelChoice] {
        self.providers.get(self.provider).map(|p| p.models.as_slice()).unwrap_or_default()
    }

    /// The provider whose models the second stage lists — also what the
    /// panel's own title names, so the developer can see which catalogue
    /// they are looking at.
    pub fn provider_id(&self) -> &str {
        self.providers.get(self.provider).map(|p| p.id.as_str()).unwrap_or_default()
    }

    pub fn selected(&self) -> usize {
        match self.stage {
            Stage::Provider => self.provider,
            Stage::Model => self.model,
        }
    }

    fn select(&mut self, index: usize) {
        match self.stage {
            // Moving off a provider invalidates the model index — it is a
            // position in *that* provider's list.
            Stage::Provider => {
                if self.provider != index {
                    self.model = 0;
                }
                self.provider = index;
            }
            Stage::Model => self.model = index,
        }
    }

    fn len(&self) -> usize {
        self.rows().len()
    }

    /// `⏎` on a provider opens its models; `⏎` on a model answers. A
    /// provider that offers no models has nothing to open, so it answers
    /// with the bare provider name — `/model <provider>` is a complete
    /// command, and the interceptor takes that provider's default.
    fn enter(&mut self) -> PickerOutcome {
        match self.stage {
            Stage::Provider => {
                if self.models().is_empty() {
                    return PickerOutcome::Chosen { provider: self.provider_id().to_string(), model: String::new() };
                }
                self.stage = Stage::Model;
                PickerOutcome::Stay
            }
            Stage::Model => match self.models().get(self.model) {
                Some(model) => {
                    PickerOutcome::Chosen { provider: self.provider_id().to_string(), model: model.id.clone() }
                }
                None => PickerOutcome::Stay,
            },
        }
    }

    /// One key. `Esc` and `←` step back a stage rather than closing outright
    /// while there is a stage to step back to: the second list is the first
    /// one narrowed, so backing out of it means reopening the question above
    /// it, not abandoning both.
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> PickerOutcome {
        if matches!(code, KeyCode::Char('c') | KeyCode::Char('d')) && modifiers.contains(KeyModifiers::CONTROL) {
            return PickerOutcome::Close;
        }
        match code {
            KeyCode::Esc | KeyCode::Left => match self.stage {
                Stage::Model => {
                    self.stage = Stage::Provider;
                    PickerOutcome::Stay
                }
                Stage::Provider => PickerOutcome::Close,
            },
            KeyCode::Up => {
                let next = self.selected().saturating_sub(1);
                self.select(next);
                PickerOutcome::Stay
            }
            KeyCode::Down => {
                // Clamped, never wrapping — the same rule first run's lists
                // follow.
                let next = (self.selected() + 1).min(self.len().saturating_sub(1));
                self.select(next);
                PickerOutcome::Stay
            }
            KeyCode::Right | KeyCode::Enter => self.enter(),
            // The decision panel's direct-pick accelerator, on the same
            // 1-based numbering its rows are drawn with.
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let index = (c as u8 - b'1') as usize;
                if index < self.len() {
                    self.select(index);
                }
                PickerOutcome::Stay
            }
            _ => PickerOutcome::Stay,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::first_run::{sample_providers, ModelChoice};

    fn picker() -> ModelPicker {
        ModelPicker::open(sample_providers(), Some("bravo"), "bravo-small").expect("a catalogue")
    }

    fn key(picker: &mut ModelPicker, code: KeyCode) -> PickerOutcome {
        picker.handle_key(code, KeyModifiers::NONE)
    }

    /// It opens where the session stands, on both halves — the list's first
    /// statement is where the developer already is.
    #[test]
    fn it_opens_on_the_provider_and_model_the_session_is_running() {
        let picker = picker();
        assert_eq!(picker.provider_id(), "bravo");
        assert_eq!(picker.selected(), 1, "the provider list opens on the row in use");
        assert_eq!(picker.model, 1, "and its model list on the model in use");
    }

    /// An endpoint the catalogue has never seen preselects nothing, and must
    /// still open rather than refusing: it is how a developer on their own
    /// host moves to a catalogue provider.
    #[test]
    fn a_provider_outside_the_catalogue_opens_on_the_first_row_and_marks_nothing() {
        let picker = ModelPicker::open(sample_providers(), None, "qwen3-coder").expect("a catalogue");
        assert_eq!(picker.selected(), 0);
        assert!(picker.rows().iter().all(|r| !r.current), "no row may claim to be one the session is not on");
    }

    #[test]
    fn an_empty_catalogue_has_nothing_to_pick_from() {
        assert_eq!(ModelPicker::open(Vec::new(), None, "m"), None);
    }

    /// The whole catalogue, not the curated prefix: `more` is first run's
    /// device for a first question, and a developer who opens this one has
    /// already had that conversation.
    #[test]
    fn the_provider_stage_lists_the_whole_catalogue() {
        assert_eq!(picker().rows().len(), sample_providers().len());
    }

    #[test]
    fn enter_opens_that_providers_models_and_enter_again_answers() {
        let mut picker = picker();
        assert_eq!(key(&mut picker, KeyCode::Enter), PickerOutcome::Stay);
        assert_eq!(picker.stage, Stage::Model);
        assert_eq!(picker.rows().iter().map(|r| r.label.clone()).collect::<Vec<_>>(), vec!["bravo-large", "bravo-small"]);
        assert_eq!(
            key(&mut picker, KeyCode::Enter),
            PickerOutcome::Chosen { provider: "bravo".into(), model: "bravo-small".into() }
        );
    }

    /// Moving off a provider must not carry its model index onto the next
    /// list, which is a different list of a different length.
    #[test]
    fn moving_the_provider_selection_resets_the_model_selection() {
        let mut picker = picker();
        key(&mut picker, KeyCode::Enter);
        key(&mut picker, KeyCode::Down); // the second model, whatever it is
        key(&mut picker, KeyCode::Esc); // back to the providers
        key(&mut picker, KeyCode::Down); // charlie
        key(&mut picker, KeyCode::Enter);
        assert_eq!(picker.model, 0, "a new provider's list opens at its top");
    }

    /// Esc backs out one stage at a time and only closes from the first —
    /// the second list is the first one narrowed.
    #[test]
    fn esc_steps_back_a_stage_before_it_closes() {
        let mut picker = picker();
        key(&mut picker, KeyCode::Enter);
        assert_eq!(key(&mut picker, KeyCode::Esc), PickerOutcome::Stay);
        assert_eq!(picker.stage, Stage::Provider);
        assert_eq!(key(&mut picker, KeyCode::Esc), PickerOutcome::Close);
    }

    #[test]
    fn ctrl_c_closes_the_picker_without_choosing() {
        let mut picker = picker();
        assert_eq!(picker.handle_key(KeyCode::Char('c'), KeyModifiers::CONTROL), PickerOutcome::Close);
    }

    #[test]
    fn selection_clamps_at_both_ends() {
        let mut picker = picker();
        for _ in 0..20 {
            key(&mut picker, KeyCode::Down);
        }
        assert_eq!(picker.selected(), sample_providers().len() - 1);
        for _ in 0..20 {
            key(&mut picker, KeyCode::Up);
        }
        assert_eq!(picker.selected(), 0);
    }

    #[test]
    fn a_digit_picks_that_row_and_out_of_range_digits_are_ignored() {
        let mut picker = picker();
        key(&mut picker, KeyCode::Char('3'));
        assert_eq!(picker.selected(), 2);
        key(&mut picker, KeyCode::Char('9'));
        assert_eq!(picker.selected(), 2);
    }

    /// A catalogue row with no models answers with the provider alone —
    /// `/model <provider>` is a complete command, so there is no dead end.
    #[test]
    fn a_provider_with_no_models_answers_with_the_provider_alone() {
        let providers = vec![ProviderChoice::new("solo", "one endpoint · SOLO_API_KEY", Vec::<ModelChoice>::new())];
        let mut picker = ModelPicker::open(providers, Some("solo"), "whatever").expect("a catalogue");
        assert_eq!(key(&mut picker, KeyCode::Enter), PickerOutcome::Chosen { provider: "solo".into(), model: String::new() });
    }

    /// The row the session is on is marked, so it stays findable once the
    /// cursor has moved off it.
    #[test]
    fn the_running_model_is_marked_on_its_own_row() {
        let mut picker = picker();
        assert_eq!(picker.rows().iter().filter(|r| r.current).count(), 1);
        key(&mut picker, KeyCode::Enter);
        let rows = picker.rows();
        let marked: Vec<&PickerRow> = rows.iter().filter(|r| r.current).collect();
        assert_eq!(marked.len(), 1);
        assert_eq!(marked[0].label, "bravo-small");
    }

    /// The same model id under a different provider is a different thing to
    /// run, and must not be marked as the one in use.
    #[test]
    fn the_current_mark_is_not_carried_onto_another_providers_list() {
        let mut picker = picker();
        key(&mut picker, KeyCode::Down); // charlie
        key(&mut picker, KeyCode::Enter);
        assert!(picker.rows().iter().all(|r| !r.current), "another provider's models are not what the session runs");
    }
}
