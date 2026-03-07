// Root App component: owns all TUI state and implements the Elm-style API.
//
// Top-level state container. Methods:
// - `apply_update()` — UiUpdate processing
// - `update()` — message dispatch (from subscription system)
// - `subscription()` — declare keybindings for the subscription system
// - `view()` — render dispatch

use std::time::Duration;

use crossterm::event::KeyCode;
use ratatui::Frame;

use crate::protocol::{AppMode, AppSnapshot, SettingsSection, UiUpdate};
use crate::tui::subscription::{Subscription, SubscriptionId};
use crate::tui::subscription::keybinding::{
    ctrl, KeyBindingRecipe, KeybindManager, PRIORITY_MODAL,
};
use crate::tui::subscription::timer::TimerRecipe;
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
use super::{BudgetStatus, LlmSetupState, StrategySetupState, TeamSummary};
use crate::tui::subscription::keybinding::KeybindHint;

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
    pub confirm_exit_settings: ConfirmDialog,
    /// Stable ID for the global Ctrl+C subscription (never changes).
    sub_id_global: SubscriptionId,
    /// Stable ID for the 500ms timer subscription (never changes).
    sub_id_tick: SubscriptionId,
    /// Monotonically incrementing counter advanced on each 500ms timer tick.
    /// Useful for blinking indicators or periodic UI refresh.
    pub tick_count: u64,
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
            confirm_exit_settings: ConfirmDialog::unsaved_changes(),
            sub_id_global: SubscriptionId::unique(),
            sub_id_tick: SubscriptionId::unique(),
            tick_count: 0,
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
                self.draft_screen.view(frame, &self.active_keybinds);
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
// AppMessage
// ---------------------------------------------------------------------------

/// Top-level messages that can be dispatched to [`App`].
#[derive(Debug, Clone)]
pub enum AppMessage {
    /// Exit the application.
    Quit,
    /// Delegate a message to the draft screen.
    Draft(DraftScreenMessage),
    /// Fired by the 500ms `TimerRecipe`. Used for blinking indicators and
    /// other periodic UI refreshes. Increments `App::tick_count`.
    Tick,
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
            AppMessage::Tick => {
                self.tick_count = self.tick_count.wrapping_add(1);
                None
            }
        }
    }

    /// Declare keybindings for the subscription system.
    ///
    /// Global Ctrl+C is registered first (highest precedence) so it fires even
    /// when a modal is open. The active screen's subscriptions follow.
    pub fn subscription(&self, kb: &mut KeybindManager) -> Subscription<AppMessage> {
        // Global: Ctrl+C → Quit (above PRIORITY_MODAL so it's always reachable).
        let global = kb.subscribe(
            KeyBindingRecipe::new(self.sub_id_global)
                .priority(PRIORITY_MODAL + 10)
                .capture()
                .bind(
                    ctrl(KeyCode::Char('c')),
                    |_| AppMessage::Quit,
                    None,
                ),
        );

        // 500ms timer — bypasses KeybindManager entirely (no hints).
        let timer_sub = TimerRecipe::new(
            self.sub_id_tick,
            Duration::from_millis(500),
            || AppMessage::Tick,
        )
        .build();

        let mode_sub = match &self.app_mode {
            AppMode::Draft => self
                .draft_screen
                .subscription(kb)
                .map(AppMessage::Draft),
            // Onboarding and Settings modes do not have subscription()
            // implementations yet — they still use handle_key().
            AppMode::Onboarding(_) | AppMode::Settings(_) => Subscription::none(),
        };

        Subscription::batch([global, timer_sub, mode_sub])
    }
}
