// Root App component: owns all TUI state and implements the Elm-style API.
//
// Replaces `ViewState` as the top-level state container. Methods:
// - `handle_key()` — input dispatch (absorbed from input.rs)
// - `apply_update()` — UiUpdate processing (absorbed from apply_ui_update)
// - `compute_keybinds()` — keybind hint computation
// - `view()` — render dispatch

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;

use crate::protocol::{AppMode, AppSnapshot, SettingsSection, UiUpdate, UserCommand};
use super::action::Action;
use super::confirm_dialog::ConfirmDialog;
use super::draft::main_panel::analysis::AnalysisPanelMessage;
use super::draft::main_panel::available::AvailablePanelMessage;
use super::draft::main_panel::MainPanelMessage;
use super::draft::sidebar::plan::PlanPanelMessage;
use super::draft::{DraftScreen, DraftScreenMessage};
use super::llm_stream::LlmStreamMessage;
use super::onboarding;
use super::settings;
use super::{BudgetStatus, KeybindHint, LlmSetupState, StrategySetupState, TeamSummary};

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub struct App {
    pub app_mode: AppMode,
    pub draft_screen: DraftScreen,
    pub active_keybinds: Vec<KeybindHint>,
    pub llm_setup: LlmSetupState,
    pub strategy_setup: StrategySetupState,
    pub settings_tab: SettingsSection,
    pub suppress_next_bracket: bool,
    pub confirm_exit_settings: ConfirmDialog,
}

impl App {
    pub fn new(initial_mode: AppMode) -> Self {
        App {
            app_mode: initial_mode,
            draft_screen: DraftScreen::new(),
            active_keybinds: Vec::new(),
            llm_setup: LlmSetupState::default(),
            strategy_setup: StrategySetupState::default(),
            settings_tab: SettingsSection::LlmConfig,
            suppress_next_bracket: false,
            confirm_exit_settings: ConfirmDialog::unsaved_changes(),
        }
    }

    // -----------------------------------------------------------------------
    // Input handling (absorbed from input.rs)
    // -----------------------------------------------------------------------

    pub fn handle_key(&mut self, key_event: KeyEvent) -> Option<UserCommand> {
        if key_event.kind != KeyEventKind::Press {
            return None;
        }

        if std::mem::take(&mut self.suppress_next_bracket)
            && key_event.code == KeyCode::Char('[')
            && key_event.modifiers == KeyModifiers::NONE
        {
            return None;
        }

        if key_event.modifiers.contains(KeyModifiers::CONTROL)
            && key_event.code == KeyCode::Char('c')
        {
            return Some(UserCommand::Quit);
        }

        let was_editing = self.is_text_editing_active();

        let result = match &self.app_mode {
            AppMode::Onboarding(_) => self.handle_onboarding_key(key_event),
            AppMode::Settings(_) => self.handle_settings_key(key_event),
            AppMode::Draft => self.draft_screen.handle_key(key_event),
        };

        if !was_editing && self.is_text_editing_active() {
            self.suppress_next_bracket = true;
        }

        result
    }

    fn is_text_editing_active(&self) -> bool {
        let strategy_editing = matches!(
            self.app_mode,
            AppMode::Onboarding(crate::onboarding::OnboardingStep::StrategySetup)
                | AppMode::Onboarding(crate::onboarding::OnboardingStep::Complete)
                | AppMode::Settings(crate::protocol::SettingsSection::StrategyConfig)
        ) && (self.strategy_setup.input_editing
            || self.strategy_setup.editing_field.is_some()
            || self.strategy_setup.overview_editing);

        self.draft_screen.main_panel.available.filter_mode()
            || self.llm_setup.api_key_editing
            || strategy_editing
            || self.draft_screen.modal_layer.position_filter.open
    }

    fn handle_onboarding_key(&mut self, key_event: KeyEvent) -> Option<UserCommand> {
        let step = match &self.app_mode {
            AppMode::Onboarding(step) => step.clone(),
            _ => return None,
        };
        let msg = onboarding::key_to_message(
            &step,
            &self.llm_setup,
            &self.strategy_setup,
            key_event,
        );
        match msg {
            Some(m) => onboarding::update(
                &step,
                &mut self.llm_setup,
                &mut self.strategy_setup,
                m,
            ),
            None => None,
        }
    }

    fn handle_settings_key(&mut self, key_event: KeyEvent) -> Option<UserCommand> {
        let msg = settings::key_to_message(
            self.settings_tab,
            &self.llm_setup,
            &self.strategy_setup,
            &self.confirm_exit_settings,
            key_event,
        );
        match msg {
            Some(m) => settings::update(
                self.settings_tab,
                &mut self.llm_setup,
                &mut self.strategy_setup,
                &mut self.confirm_exit_settings,
                m,
            ),
            None => None,
        }
    }

    // -----------------------------------------------------------------------
    // UiUpdate processing (absorbed from apply_ui_update)
    // -----------------------------------------------------------------------

    pub fn apply_update(&mut self, update: UiUpdate) {
        match update {
            UiUpdate::StateSnapshot(snapshot) => {
                self.apply_snapshot(*snapshot);
            }
            UiUpdate::NominationUpdate(nomination) => {
                self.draft_screen.current_nomination = Some(*nomination);
                self.draft_screen.main_panel.analysis.update(AnalysisPanelMessage::Stream(LlmStreamMessage::Clear));
                self.draft_screen.instant_analysis = None;
                self.draft_screen.focused_panel = None;
                self.draft_screen.main_panel.available.update(AvailablePanelMessage::Scroll(
                    crate::tui::scroll::ScrollDirection::Top,
                ));
            }
            UiUpdate::BidUpdate(nomination) => {
                self.draft_screen.current_nomination = Some(*nomination);
            }
            UiUpdate::NominationCleared => {
                self.draft_screen.current_nomination = None;
                self.draft_screen.instant_analysis = None;
                self.draft_screen.main_panel.analysis.update(AnalysisPanelMessage::Stream(LlmStreamMessage::Clear));
                self.draft_screen.focused_panel = None;
            }
            UiUpdate::AnalysisToken(token) => {
                self.draft_screen.main_panel.analysis.update(AnalysisPanelMessage::Stream(
                    LlmStreamMessage::TokenReceived(token),
                ));
            }
            UiUpdate::AnalysisComplete(final_text) => {
                self.draft_screen.main_panel.analysis.update(AnalysisPanelMessage::Stream(
                    LlmStreamMessage::Complete(final_text),
                ));
            }
            UiUpdate::AnalysisError(msg) => {
                self.draft_screen.main_panel.analysis.update(AnalysisPanelMessage::Stream(
                    LlmStreamMessage::Error(msg),
                ));
            }
            UiUpdate::PlanStarted => {
                self.draft_screen.sidebar.plan.update(PlanPanelMessage::Stream(LlmStreamMessage::Clear));
                self.draft_screen.sidebar.plan.update(PlanPanelMessage::Stream(
                    LlmStreamMessage::TokenReceived(String::new()),
                ));
            }
            UiUpdate::PlanToken(token) => {
                self.draft_screen.sidebar.plan.update(PlanPanelMessage::Stream(
                    LlmStreamMessage::TokenReceived(token),
                ));
            }
            UiUpdate::PlanComplete(final_text) => {
                self.draft_screen.sidebar.plan.update(PlanPanelMessage::Stream(
                    LlmStreamMessage::Complete(final_text),
                ));
            }
            UiUpdate::PlanError(msg) => {
                self.draft_screen.sidebar.plan.update(PlanPanelMessage::Stream(
                    LlmStreamMessage::Error(msg),
                ));
            }
            UiUpdate::ConnectionStatus(status) => {
                self.draft_screen.connection_status = status;
            }
            UiUpdate::OnboardingUpdate(update) => {
                use crate::protocol::OnboardingUpdate;
                use crate::llm::provider::models_for_provider;
                use onboarding::llm_setup::LlmConnectionStatus;

                match update {
                    OnboardingUpdate::ConnectionTestResult { success, message } => {
                        self.llm_setup.connection_status = if success {
                            self.llm_setup.settings_needs_connection_test = false;
                            LlmConnectionStatus::Success(message)
                        } else {
                            LlmConnectionStatus::Failed(message)
                        };
                    }
                    OnboardingUpdate::ProgressSync { provider, model, api_key_mask } => {
                        let in_settings = self.llm_setup.in_settings_mode;

                        if let Some(ref p) = provider {
                            if let Some(idx) = LlmSetupState::PROVIDERS.iter().position(|pp| pp == p) {
                                self.llm_setup.selected_provider_idx = idx;
                            }
                            if !in_settings {
                                self.llm_setup.confirmed_through =
                                    Some(onboarding::llm_setup::LlmSetupSection::Provider);
                                self.llm_setup.active_section =
                                    onboarding::llm_setup::LlmSetupSection::Model;
                            }
                            if let Some(ref model_id) = model {
                                let models = models_for_provider(p);
                                if let Some(midx) = models.iter().position(|m| m.model_id == model_id.as_str()) {
                                    self.llm_setup.selected_model_idx = midx;
                                }
                                if !in_settings {
                                    self.llm_setup.confirmed_through =
                                        Some(onboarding::llm_setup::LlmSetupSection::Model);
                                    self.llm_setup.active_section =
                                        onboarding::llm_setup::LlmSetupSection::ApiKey;
                                }
                            }
                        }
                        if let Some(mask) = api_key_mask {
                            self.llm_setup.has_saved_api_key = true;
                            self.llm_setup.saved_api_key_mask = mask;
                        } else {
                            self.llm_setup.has_saved_api_key = false;
                            self.llm_setup.saved_api_key_mask.clear();
                        }
                    }
                    OnboardingUpdate::StrategyLlmToken(token) => {
                        self.strategy_setup.generation_output.push_str(&token);
                    }
                    OnboardingUpdate::StrategyLlmComplete { hitting_budget_pct, category_weights, strategy_overview } => {
                        let was_generating = self.strategy_setup.generating;
                        self.strategy_setup.generating = false;
                        self.strategy_setup.generation_error = None;
                        self.strategy_setup.hitting_budget_pct = hitting_budget_pct;
                        self.strategy_setup.category_weights = category_weights;
                        self.strategy_setup.strategy_overview = strategy_overview;
                        self.strategy_setup.step = onboarding::strategy_setup::StrategyWizardStep::Review;
                        self.strategy_setup.review_section = onboarding::strategy_setup::ReviewSection::Overview;
                        self.strategy_setup.input_editing = false;
                        if was_generating {
                            self.strategy_setup.settings_dirty = true;
                        } else if matches!(self.app_mode, AppMode::Settings(_)) {
                            self.strategy_setup.settings_dirty = false;
                            self.strategy_setup.snapshot_settings();
                        }
                    }
                    OnboardingUpdate::StrategyLlmError(msg) => {
                        self.strategy_setup.generating = false;
                        self.strategy_setup.generation_error = Some(msg);
                    }
                }
            }
            UiUpdate::ModeChanged(mode) => {
                self.confirm_exit_settings.open = false;
                if let AppMode::Settings(section) = &mode {
                    self.settings_tab = *section;
                    self.llm_setup.confirmed_through =
                        Some(onboarding::llm_setup::LlmSetupSection::ApiKey);
                    self.llm_setup.active_section =
                        onboarding::llm_setup::LlmSetupSection::Provider;
                    self.llm_setup.settings_editing_field = None;
                    self.llm_setup.settings_dirty = false;
                    self.llm_setup.in_settings_mode = true;
                    self.llm_setup.snapshot_settings();

                    self.strategy_setup.settings_dirty = false;
                    self.strategy_setup.overview_editing = false;
                    self.strategy_setup.overview_input.clear();
                    self.strategy_setup.snapshot_settings();
                } else {
                    self.llm_setup.in_settings_mode = false;
                }
                self.app_mode = mode;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Snapshot application
    // -----------------------------------------------------------------------

    pub fn apply_snapshot(&mut self, snapshot: AppSnapshot) {
        self.app_mode = snapshot.app_mode;
        let ds = &mut self.draft_screen;
        ds.pick_number = snapshot.pick_count;
        ds.total_picks = snapshot.total_picks;
        if let Some(tab) = snapshot.active_tab {
            if ds.main_panel.active_tab() != tab {
                ds.focused_panel = None;
            }
            ds.main_panel.update(MainPanelMessage::SwitchTab(tab));
        }

        ds.available_players = snapshot.available_players;
        ds.positional_scarcity = snapshot.positional_scarcity;
        ds.draft_log = snapshot.draft_log;
        ds.my_roster = snapshot.my_roster;

        ds.budget = BudgetStatus {
            spent: snapshot.budget_spent,
            remaining: snapshot.budget_remaining,
            cap: snapshot.salary_cap,
            inflation_rate: snapshot.inflation_rate,
            max_bid: snapshot.max_bid,
            avg_per_slot: snapshot.avg_per_slot,
        };

        ds.inflation = snapshot.inflation_rate;

        ds.team_summaries = snapshot
            .team_snapshots
            .into_iter()
            .map(|ts| TeamSummary {
                name: ts.name,
                budget_remaining: ts.budget_remaining,
                slots_filled: ts.slots_filled,
                total_slots: ts.total_slots,
            })
            .collect();

        ds.llm_configured = snapshot.llm_configured;
    }

    pub fn settings_is_editing(&self) -> bool {
        match self.settings_tab {
            SettingsSection::LlmConfig => {
                self.llm_setup.api_key_editing || self.llm_setup.is_settings_field_editing()
            }
            SettingsSection::StrategyConfig => self.strategy_setup.is_editing(),
        }
    }

    // -----------------------------------------------------------------------
    // Keybind computation
    // -----------------------------------------------------------------------

    pub fn compute_keybinds(&self) -> Vec<KeybindHint> {
        match &self.app_mode {
            AppMode::Onboarding(step) => self.compute_onboarding_keybinds(step),
            AppMode::Settings(_) => self.compute_settings_keybinds(),
            AppMode::Draft => self.draft_screen.compute_keybinds(),
        }
    }

    fn compute_onboarding_keybinds(&self, step: &crate::onboarding::OnboardingStep) -> Vec<KeybindHint> {
        use crate::onboarding::OnboardingStep;
        use onboarding::llm_setup::LlmSetupSection;

        match step {
            OnboardingStep::LlmSetup => {
                if self.llm_setup.api_key_editing {
                    vec![
                        KeybindHint::new("type", "Input key"),
                        KeybindHint::new("Enter", "Confirm & test"),
                        KeybindHint::new("Esc", "Back"),
                    ]
                } else {
                    let mut hints = Vec::new();
                    match self.llm_setup.active_section {
                        LlmSetupSection::Provider | LlmSetupSection::Model => {
                            hints.push(KeybindHint::new("^v", "Select"));
                            hints.push(KeybindHint::new("Enter", "Confirm"));
                        }
                        LlmSetupSection::ApiKey => {
                            if self.llm_setup.connection_tested_ok() {
                                hints.push(KeybindHint::new("Enter", "Continue"));
                            } else if self.llm_setup.api_key_input.is_empty() && !self.llm_setup.has_saved_api_key {
                                hints.push(KeybindHint::new("Enter", "Input key"));
                            } else if self.llm_setup.api_key_input.is_empty() && self.llm_setup.has_saved_api_key {
                                hints.push(KeybindHint::new("Enter", "Edit key"));
                            } else if matches!(
                                self.llm_setup.connection_status,
                                onboarding::llm_setup::LlmConnectionStatus::Failed(_)
                            ) {
                                hints.push(KeybindHint::new("Enter", "Edit key"));
                            } else if matches!(
                                self.llm_setup.connection_status,
                                onboarding::llm_setup::LlmConnectionStatus::Testing
                            ) {
                                hints.push(KeybindHint::new("...", "Testing"));
                            } else {
                                hints.push(KeybindHint::new("Enter", "Test connection"));
                            }
                        }
                    }
                    if self.llm_setup.active_section != LlmSetupSection::Provider {
                        hints.push(KeybindHint::new("Esc", "Back"));
                    }
                    if self.llm_setup.connection_tested_ok() {
                        hints.push(KeybindHint::new("n", "Continue ->"));
                    }
                    hints.push(KeybindHint::new("s", "Skip"));
                    hints
                }
            }
            OnboardingStep::StrategySetup | OnboardingStep::Complete => {
                use onboarding::strategy_setup::StrategyWizardStep;
                let ss = &self.strategy_setup;

                match ss.step {
                    StrategyWizardStep::Input => {
                        if ss.input_editing {
                            vec![
                                KeybindHint::new("type", "Describe strategy"),
                                KeybindHint::new("Enter", "Generate"),
                                KeybindHint::new("Esc", "Stop editing"),
                            ]
                        } else {
                            vec![
                                KeybindHint::new("e", "Edit text"),
                                KeybindHint::new("Enter", "Generate"),
                                KeybindHint::new("Esc", "Back"),
                            ]
                        }
                    }
                    StrategyWizardStep::Generating => {
                        if ss.generation_error.is_some() {
                            vec![
                                KeybindHint::new("Enter", "Retry"),
                                KeybindHint::new("Esc", "Back"),
                            ]
                        } else {
                            vec![KeybindHint::new("", "Generating...")]
                        }
                    }
                    StrategyWizardStep::Review => {
                        if ss.generating {
                            if ss.generation_error.is_some() {
                                vec![
                                    KeybindHint::new("Enter", "Retry"),
                                    KeybindHint::new("Esc", "Cancel"),
                                ]
                            } else {
                                vec![
                                    KeybindHint::new("", "Generating..."),
                                    KeybindHint::new("Esc", "Cancel"),
                                ]
                            }
                        } else if ss.overview_editing {
                            vec![
                                KeybindHint::new("type", "Edit overview"),
                                KeybindHint::new("Enter", "Submit to AI"),
                                KeybindHint::new("Esc", "Cancel"),
                            ]
                        } else if ss.editing_field.is_some() {
                            vec![
                                KeybindHint::new("type", "Enter value"),
                                KeybindHint::new("Enter", "Confirm"),
                                KeybindHint::new("Esc", "Cancel"),
                            ]
                        } else {
                            let mut hints = Vec::new();
                            hints.push(KeybindHint::new("Enter", "Edit"));
                            hints.push(KeybindHint::new("s", "Save"));
                            hints.push(KeybindHint::new("Esc", "Back"));
                            hints
                        }
                    }
                    StrategyWizardStep::Confirm => {
                        vec![
                            KeybindHint::new("<>", "Yes / No"),
                            KeybindHint::new("Enter", "Confirm"),
                            KeybindHint::new("Esc", "Back"),
                        ]
                    }
                }
            }
        }
    }

    fn compute_settings_keybinds(&self) -> Vec<KeybindHint> {
        if self.confirm_exit_settings.open {
            return vec![
                KeybindHint::new("y", "Save & exit"),
                KeybindHint::new("n", "Discard & exit"),
                KeybindHint::new("Esc", "Cancel"),
            ];
        }

        if self.settings_tab == SettingsSection::StrategyConfig {
            let ss = &self.strategy_setup;
            if ss.generating {
                return if ss.generation_error.is_some() {
                    vec![
                        KeybindHint::new("Enter", "Retry"),
                        KeybindHint::new("Esc", "Cancel"),
                    ]
                } else {
                    vec![
                        KeybindHint::new("", "Generating..."),
                        KeybindHint::new("Esc", "Cancel"),
                    ]
                };
            }
            if ss.generation_error.is_some() {
                return vec![
                    KeybindHint::new("Enter", "Retry"),
                    KeybindHint::new("Esc", "Back"),
                ];
            }
            if ss.overview_editing {
                return vec![
                    KeybindHint::new("type", "Edit overview"),
                    KeybindHint::new("Enter", "Submit to AI"),
                    KeybindHint::new("Esc", "Cancel"),
                ];
            }
        }

        if self.settings_is_editing() {
            let is_llm_dropdown = self.settings_tab == SettingsSection::LlmConfig
                && matches!(
                    self.llm_setup.settings_editing_field,
                    Some(
                        crate::tui::onboarding::llm_setup::LlmSetupSection::Provider
                            | crate::tui::onboarding::llm_setup::LlmSetupSection::Model
                    )
                );
            if is_llm_dropdown {
                vec![
                    KeybindHint::new("\u{2191}\u{2193}", "Select"),
                    KeybindHint::new("Enter", "Confirm"),
                    KeybindHint::new("Esc", "Cancel"),
                ]
            } else {
                vec![
                    KeybindHint::new("type", "Input"),
                    KeybindHint::new("Enter", "Confirm"),
                    KeybindHint::new("Esc", "Cancel"),
                ]
            }
        } else {
            let mut hints = vec![
                KeybindHint::new("1/2", "Tab"),
                KeybindHint::new("Tab", "Section"),
                KeybindHint::new("^v", "Navigate"),
            ];
            match self.settings_tab {
                SettingsSection::StrategyConfig => {
                    hints.push(KeybindHint::new("Enter", "Edit"));
                    hints.push(KeybindHint::new("s", "Save"));
                    if self.strategy_setup.settings_dirty {
                        hints.push(KeybindHint::new("", "[unsaved]"));
                    }
                }
                SettingsSection::LlmConfig => {
                    hints.push(KeybindHint::new("Enter", "Edit"));
                    if !self.llm_setup.is_save_blocked() && self.llm_setup.settings_dirty {
                        hints.push(KeybindHint::new("s", "Save"));
                    }
                    if self.llm_setup.settings_dirty
                        || self.llm_setup.settings_needs_connection_test
                    {
                        hints.push(KeybindHint::new("", "[unsaved]"));
                    }
                }
            }
            hints.push(KeybindHint::new("Esc", "Back to Draft"));
            hints
        }
    }

    // -----------------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------------

    pub fn view(&self, frame: &mut Frame) {
        match &self.app_mode {
            AppMode::Onboarding(step) => {
                onboarding::render(frame, step, self);
            }
            AppMode::Settings(_section) => {
                settings::render(frame, self);
            }
            AppMode::Draft => {
                self.draft_screen.view(frame);
            }
        }
    }
}

impl Default for App {
    fn default() -> Self {
        App::new(AppMode::Draft)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::pick::Position;
    use crate::protocol::{OnboardingAction, TabId};
    use crate::tui::FocusPanel;
    use crate::tui::draft::main_panel::MainPanelMessage;
    use crate::tui::draft::main_panel::analysis::AnalysisPanelMessage;
    use crate::tui::draft::main_panel::available::AvailablePanelMessage;
    use crate::tui::draft::modal::position_filter::PositionFilterModalMessage;
    use crate::tui::scroll::ScrollDirection;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    /// Helper to create a KeyEvent with no modifiers.
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    /// Helper to create a KeyEvent with Ctrl modifier.
    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    // -- Tab switching --

    #[test]
    fn tab_1_switches_to_analysis() {
        let mut app = App::default();
        app.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Teams));
        let result = app.handle_key(key(KeyCode::Char('1')));
        assert!(result.is_none());
        assert_eq!(app.draft_screen.main_panel.active_tab(), TabId::Analysis);
    }

    #[test]
    fn tab_2_switches_to_available() {
        let mut app = App::default();
        let result = app.handle_key(key(KeyCode::Char('2')));
        assert!(result.is_none());
        assert_eq!(app.draft_screen.main_panel.active_tab(), TabId::Available);
    }

    #[test]
    fn tab_3_switches_to_draft_log() {
        let mut app = App::default();
        let result = app.handle_key(key(KeyCode::Char('3')));
        assert!(result.is_none());
        assert_eq!(app.draft_screen.main_panel.active_tab(), TabId::DraftLog);
    }

    #[test]
    fn tab_4_switches_to_teams() {
        let mut app = App::default();
        let result = app.handle_key(key(KeyCode::Char('4')));
        assert!(result.is_none());
        assert_eq!(app.draft_screen.main_panel.active_tab(), TabId::Teams);
    }

    // -- Scroll --

    #[test]
    fn arrow_up_decrements_scroll() {
        let mut app = App::default();
        for _ in 0..5 {
            app.draft_screen.main_panel.analysis.update(AnalysisPanelMessage::Scroll(ScrollDirection::Down));
        }
        let result = app.handle_key(key(KeyCode::Up));
        assert!(result.is_none());
        assert_eq!(app.draft_screen.main_panel.analysis.scroll_offset(), 4);
    }

    #[test]
    fn arrow_down_increments_scroll() {
        let mut app = App::default();
        let result = app.handle_key(key(KeyCode::Down));
        assert!(result.is_none());
        assert_eq!(app.draft_screen.main_panel.analysis.scroll_offset(), 1);
    }

    #[test]
    fn k_scrolls_up() {
        let mut app = App::default();
        for _ in 0..3 {
            app.draft_screen.main_panel.analysis.update(AnalysisPanelMessage::Scroll(ScrollDirection::Down));
        }
        let result = app.handle_key(key(KeyCode::Char('k')));
        assert!(result.is_none());
        assert_eq!(app.draft_screen.main_panel.analysis.scroll_offset(), 2);
    }

    #[test]
    fn j_scrolls_down() {
        let mut app = App::default();
        let result = app.handle_key(key(KeyCode::Char('j')));
        assert!(result.is_none());
        assert_eq!(app.draft_screen.main_panel.analysis.scroll_offset(), 1);
    }

    #[test]
    fn scroll_up_does_not_underflow() {
        let mut app = App::default();
        let result = app.handle_key(key(KeyCode::Up));
        assert!(result.is_none());
        assert_eq!(app.draft_screen.main_panel.analysis.scroll_offset(), 0);
    }

    #[test]
    fn page_down_scrolls_by_page_size() {
        let mut app = App::default();
        let result = app.handle_key(key(KeyCode::PageDown));
        assert!(result.is_none());
        assert_eq!(app.draft_screen.main_panel.analysis.scroll_offset(), 20);
    }

    #[test]
    fn page_up_scrolls_by_page_size() {
        let mut app = App::default();
        for _ in 0..25 {
            app.draft_screen.main_panel.analysis.update(AnalysisPanelMessage::Scroll(ScrollDirection::Down));
        }
        let result = app.handle_key(key(KeyCode::PageUp));
        assert!(result.is_none());
        assert_eq!(app.draft_screen.main_panel.analysis.scroll_offset(), 5);
    }

    #[test]
    fn scroll_applies_to_active_tab_widget() {
        let mut app = App::default();
        app.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.draft_screen.main_panel.available.scroll_offset(), 2);
        assert_eq!(app.draft_screen.main_panel.analysis.scroll_offset(), 0);
        assert_eq!(app.draft_screen.sidebar.plan.scroll_offset(), 0);
    }

    // -- Panel focus --

    #[test]
    fn tab_cycles_focus_forward() {
        let mut app = App::default();
        assert!(app.draft_screen.focused_panel.is_none());

        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.draft_screen.focused_panel, Some(FocusPanel::MainPanel));

        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.draft_screen.focused_panel, Some(FocusPanel::Roster));

        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.draft_screen.focused_panel, Some(FocusPanel::Scarcity));

        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.draft_screen.focused_panel, Some(FocusPanel::Budget));

        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.draft_screen.focused_panel, Some(FocusPanel::NominationPlan));

        app.handle_key(key(KeyCode::Tab));
        assert!(app.draft_screen.focused_panel.is_none());
    }

    #[test]
    fn backtab_cycles_focus_backward() {
        let mut app = App::default();
        assert!(app.draft_screen.focused_panel.is_none());

        app.handle_key(key(KeyCode::BackTab));
        assert_eq!(app.draft_screen.focused_panel, Some(FocusPanel::NominationPlan));

        app.handle_key(key(KeyCode::BackTab));
        assert_eq!(app.draft_screen.focused_panel, Some(FocusPanel::Budget));

        app.handle_key(key(KeyCode::BackTab));
        assert_eq!(app.draft_screen.focused_panel, Some(FocusPanel::Scarcity));

        app.handle_key(key(KeyCode::BackTab));
        assert_eq!(app.draft_screen.focused_panel, Some(FocusPanel::Roster));

        app.handle_key(key(KeyCode::BackTab));
        assert_eq!(app.draft_screen.focused_panel, Some(FocusPanel::MainPanel));

        app.handle_key(key(KeyCode::BackTab));
        assert!(app.draft_screen.focused_panel.is_none());
    }

    #[test]
    fn shift_tab_cycles_focus_backward() {
        let mut app = App::default();
        assert!(app.draft_screen.focused_panel.is_none());

        let shift_tab = KeyEvent {
            code: KeyCode::Tab,
            modifiers: KeyModifiers::SHIFT,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };

        app.handle_key(shift_tab);
        assert_eq!(app.draft_screen.focused_panel, Some(FocusPanel::NominationPlan));

        app.handle_key(shift_tab);
        assert_eq!(app.draft_screen.focused_panel, Some(FocusPanel::Budget));

        app.handle_key(shift_tab);
        assert_eq!(app.draft_screen.focused_panel, Some(FocusPanel::Scarcity));

        app.handle_key(shift_tab);
        assert_eq!(app.draft_screen.focused_panel, Some(FocusPanel::Roster));

        app.handle_key(shift_tab);
        assert_eq!(app.draft_screen.focused_panel, Some(FocusPanel::MainPanel));

        app.handle_key(shift_tab);
        assert!(app.draft_screen.focused_panel.is_none());
    }

    #[test]
    fn esc_clears_focus() {
        let mut app = App::default();
        app.draft_screen.focused_panel = Some(FocusPanel::MainPanel);

        app.handle_key(key(KeyCode::Esc));
        assert!(app.draft_screen.focused_panel.is_none());
    }

    #[test]
    fn scroll_routes_to_roster_when_focused() {
        let mut app = App::default();
        app.draft_screen.focused_panel = Some(FocusPanel::Roster);

        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Down));

        assert_eq!(app.draft_screen.sidebar.roster.scroll_offset(), 2);
        assert_eq!(app.draft_screen.main_panel.analysis.scroll_offset(), 0);
    }

    #[test]
    fn scroll_routes_to_scarcity_when_focused() {
        let mut app = App::default();
        app.draft_screen.focused_panel = Some(FocusPanel::Scarcity);

        app.handle_key(key(KeyCode::Down));

        assert_eq!(app.draft_screen.sidebar.scarcity.scroll_offset(), 1);
        assert_eq!(app.draft_screen.main_panel.analysis.scroll_offset(), 0);
    }

    #[test]
    fn scroll_routes_to_budget_when_focused() {
        let mut app = App::default();
        app.draft_screen.focused_panel = Some(FocusPanel::Budget);

        app.handle_key(key(KeyCode::Down));

        assert_eq!(app.draft_screen.scroll_offset.get("budget"), Some(&1));
        assert_eq!(app.draft_screen.main_panel.analysis.scroll_offset(), 0);
    }

    #[test]
    fn scroll_routes_to_nom_plan_when_focused() {
        let mut app = App::default();
        app.draft_screen.focused_panel = Some(FocusPanel::NominationPlan);

        app.handle_key(key(KeyCode::Down));

        assert_eq!(app.draft_screen.sidebar.plan.scroll_offset(), 1);
        assert_eq!(app.draft_screen.main_panel.analysis.scroll_offset(), 0);
    }

    #[test]
    fn scroll_routes_to_main_when_focused() {
        let mut app = App::default();
        app.draft_screen.focused_panel = Some(FocusPanel::MainPanel);

        app.handle_key(key(KeyCode::Down));

        assert_eq!(app.draft_screen.main_panel.analysis.scroll_offset(), 1);
        assert!(app.draft_screen.scroll_offset.get("sidebar").is_none());
    }

    #[test]
    fn scroll_routes_to_main_when_no_focus() {
        let mut app = App::default();
        assert!(app.draft_screen.focused_panel.is_none());

        app.handle_key(key(KeyCode::Down));

        assert_eq!(app.draft_screen.main_panel.analysis.scroll_offset(), 1);
        assert!(app.draft_screen.scroll_offset.get("sidebar").is_none());
    }

    #[test]
    fn page_scroll_routes_to_roster_when_focused() {
        let mut app = App::default();
        app.draft_screen.focused_panel = Some(FocusPanel::Roster);

        app.handle_key(key(KeyCode::PageDown));

        assert_eq!(app.draft_screen.sidebar.roster.scroll_offset(), 20);
        assert_eq!(app.draft_screen.main_panel.analysis.scroll_offset(), 0);
    }

    #[test]
    fn tab_does_not_affect_other_state() {
        let mut app = App::default();
        app.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));

        app.handle_key(key(KeyCode::Tab));

        assert_eq!(app.draft_screen.main_panel.active_tab(), TabId::Available, "Tab should not switch tabs");
        assert!(!app.draft_screen.main_panel.available.filter_mode(), "Tab should not enter filter mode");
    }

    #[test]
    fn tab_switch_clears_focused_panel() {
        for (key_char, expected_tab) in [
            ('1', TabId::Analysis),
            ('2', TabId::Available),
            ('3', TabId::DraftLog),
            ('4', TabId::Teams),
        ] {
            let mut app = App::default();
            app.draft_screen.focused_panel = Some(FocusPanel::MainPanel);
            app.handle_key(key(KeyCode::Char(key_char)));
            assert_eq!(app.draft_screen.main_panel.active_tab(), expected_tab, "Key '{}' should switch to {:?}", key_char, expected_tab);
            assert!(
                app.draft_screen.focused_panel.is_none(),
                "Key '{}': focused_panel should be None after tab switch, got {:?}",
                key_char,
                app.draft_screen.focused_panel
            );
        }
    }

    #[test]
    fn tab_switch_clears_sidebar_focused_panel() {
        for focused in [
            FocusPanel::Roster,
            FocusPanel::Scarcity,
            FocusPanel::Budget,
            FocusPanel::NominationPlan,
        ] {
            let mut app = App::default();
            app.draft_screen.focused_panel = Some(focused);
            app.handle_key(key(KeyCode::Char('2')));
            assert!(
                app.draft_screen.focused_panel.is_none(),
                "focused_panel {:?} should be cleared after tab switch",
                focused
            );
        }
    }

    // -- Filter mode --

    #[test]
    fn slash_enters_filter_mode_on_available_tab() {
        let mut app = App::default();
        app.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));
        let result = app.handle_key(key(KeyCode::Char('/')));
        assert!(result.is_none());
        assert!(app.draft_screen.main_panel.available.filter_mode());
    }

    #[test]
    fn slash_does_not_enter_filter_mode_on_other_tabs() {
        for tab in [TabId::Analysis, TabId::DraftLog, TabId::Teams] {
            let mut app = App::default();
            app.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(tab));
            let result = app.handle_key(key(KeyCode::Char('/')));
            assert!(result.is_none(), "/ on {:?} should return None", tab);
            assert!(
                !app.draft_screen.main_panel.available.filter_mode(),
                "/ on {:?} should not activate filter_mode",
                tab
            );
        }
    }

    #[test]
    fn filter_mode_captures_keys() {
        let mut app = App::default();
        app.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));
        app.draft_screen.main_panel.available.update(AvailablePanelMessage::ToggleFilterMode);

        let result = app.handle_key(key(KeyCode::Char('a')));
        assert!(result.is_none(), "filter mode should capture letter keys");

        let result = app.handle_key(key(KeyCode::Char('1')));
        assert!(result.is_none(), "filter mode should capture number keys");
        assert_eq!(app.draft_screen.main_panel.active_tab(), TabId::Available, "tab should not change during filter mode");
    }

    #[test]
    fn esc_exits_filter_mode() {
        let mut app = App::default();
        app.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));
        app.draft_screen.main_panel.available.update(AvailablePanelMessage::ToggleFilterMode);
        assert!(app.draft_screen.main_panel.available.filter_mode());

        app.handle_key(key(KeyCode::Esc));
        assert!(!app.draft_screen.main_panel.available.filter_mode());
    }

    // -- Position filter --

    #[test]
    fn p_opens_position_filter_on_available_tab() {
        let mut app = App::default();
        app.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));
        let result = app.handle_key(key(KeyCode::Char('p')));
        assert!(result.is_none());
        assert!(app.draft_screen.modal_layer.position_filter.open);
    }

    #[test]
    fn p_does_not_open_position_filter_on_other_tabs() {
        for tab in [TabId::Analysis, TabId::DraftLog, TabId::Teams] {
            let mut app = App::default();
            app.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(tab));
            let result = app.handle_key(key(KeyCode::Char('p')));
            assert!(result.is_none());
            assert!(
                !app.draft_screen.modal_layer.position_filter.open,
                "p on {:?} should not open position filter",
                tab
            );
        }
    }

    #[test]
    fn position_filter_modal_captures_keys() {
        let mut app = App::default();
        app.draft_screen.modal_layer.position_filter.update(
            PositionFilterModalMessage::Open { current_filter: None },
        );
        assert!(app.draft_screen.modal_layer.position_filter.open);

        let result = app.handle_key(key(KeyCode::Char('1')));
        assert!(result.is_none(), "modal should capture all keys");
        assert_eq!(app.draft_screen.main_panel.active_tab(), TabId::Analysis, "tabs should not switch while modal is open");
    }

    // -- Quit / Ctrl+C --

    #[test]
    fn ctrl_c_returns_quit() {
        let mut app = App::default();
        let result = app.handle_key(ctrl_key(KeyCode::Char('c')));
        assert_eq!(result, Some(UserCommand::Quit));
    }

    #[test]
    fn q_enters_quit_confirm_mode() {
        let mut app = App::default();
        let result = app.handle_key(key(KeyCode::Char('q')));
        assert!(result.is_none(), "q should not immediately quit");
        assert!(app.draft_screen.modal_layer.quit_confirm.open, "q should open quit confirmation");
    }

    #[test]
    fn quit_confirm_y_quits() {
        let mut app = App::default();
        app.handle_key(key(KeyCode::Char('q')));
        assert!(app.draft_screen.modal_layer.quit_confirm.open);

        let result = app.handle_key(key(KeyCode::Char('y')));
        assert_eq!(result, Some(UserCommand::Quit));
    }

    #[test]
    fn quit_confirm_n_cancels() {
        let mut app = App::default();
        app.handle_key(key(KeyCode::Char('q')));
        assert!(app.draft_screen.modal_layer.quit_confirm.open);

        let result = app.handle_key(key(KeyCode::Char('n')));
        assert!(result.is_none());
        assert!(!app.draft_screen.modal_layer.quit_confirm.open);
    }

    #[test]
    fn quit_confirm_blocks_other_keys() {
        let mut app = App::default();
        app.handle_key(key(KeyCode::Char('q')));
        assert!(app.draft_screen.modal_layer.quit_confirm.open);

        let result = app.handle_key(key(KeyCode::Char('2')));
        assert!(result.is_none(), "quit confirm should block tab switching");
        assert_eq!(app.draft_screen.main_panel.active_tab(), TabId::Analysis, "tab should not change during quit confirm");
    }

    // -- Special keys --

    #[test]
    fn r_requests_keyframe() {
        let mut app = App::default();
        let result = app.handle_key(key(KeyCode::Char('r')));
        assert_eq!(result, Some(UserCommand::RequestKeyframe));
    }

    #[test]
    fn comma_opens_settings() {
        let mut app = App::default();
        let result = app.handle_key(key(KeyCode::Char(',')));
        assert_eq!(result, Some(UserCommand::OpenSettings));
    }

    // -- Release events ignored --

    #[test]
    fn release_events_are_ignored() {
        let mut app = App::default();
        let release_event = KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        };
        let result = app.handle_key(release_event);
        assert!(result.is_none(), "release events should be ignored");
        assert!(!app.draft_screen.modal_layer.quit_confirm.open, "release should not trigger quit confirm");
    }

    // -- Bracket suppression --

    #[test]
    fn bracket_suppressed_after_entering_filter_mode() {
        let mut app = App::default();
        app.draft_screen.main_panel.update(MainPanelMessage::SwitchTab(TabId::Available));

        app.handle_key(key(KeyCode::Char('/')));
        assert!(app.suppress_next_bracket, "entering filter mode should set bracket suppression");

        let result = app.handle_key(key(KeyCode::Char('[')));
        assert!(result.is_none(), "suppressed bracket should return None");
        assert!(!app.suppress_next_bracket, "bracket suppression should be cleared after use");
    }

    #[test]
    fn bracket_not_suppressed_when_not_entering_edit_mode() {
        let mut app = App::default();
        assert!(!app.suppress_next_bracket);

        app.handle_key(key(KeyCode::Down));
        assert!(!app.suppress_next_bracket, "non-edit mode keys should not set suppression");
    }
}

// ---------------------------------------------------------------------------
// AppMessage
// ---------------------------------------------------------------------------

/// Top-level messages that can be dispatched to [`App`].
///
/// This enum is the entry point for the message-based input path. The
/// existing [`App::handle_key`] system is untouched — both systems coexist.
/// `AppMessage` will be produced by `subscription()` in later phases.
#[derive(Debug, Clone)]
pub enum AppMessage {
    /// Exit the application.
    Quit,
    /// Delegate a message to the draft screen.
    Draft(DraftScreenMessage),
}

impl App {
    /// Process a [`AppMessage`] and return an optional [`Action`].
    ///
    /// Routes messages to the appropriate sub-component. Returns `Some(Action)`
    /// when the message requires the event loop to take an action (e.g. quit or
    /// send a backend command). Returns `None` when the message was handled
    /// internally with no upward effect.
    pub fn update(&mut self, msg: AppMessage) -> Option<Action> {
        match msg {
            AppMessage::Quit => Some(Action::Quit),
            AppMessage::Draft(m) => self.draft_screen.update(m),
        }
    }
}
