// LLM setup screen: provider, model, and API key configuration.
//
// This is Step 1 of the onboarding wizard. The user selects an LLM provider,
// chooses a model, enters an API key, and optionally tests the connection
// before proceeding to strategy configuration.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crossterm::event::{KeyCode, KeyEvent};

use crate::llm::provider::{models_for_provider, LlmProvider, ModelOption};
use crate::protocol::{OnboardingAction, SettingsSection, UserCommand};
use crate::tui::TextInput;
use crate::tui::subscription::{Subscription, SubscriptionId};
use crate::tui::subscription::keybinding::{
    exact, KeyBindingRecipe, KeybindHint, KeybindManager, KeyTrigger, PRIORITY_CAPTURE,
    PRIORITY_NORMAL,
};

// ---------------------------------------------------------------------------
// API key masking
// ---------------------------------------------------------------------------

/// Build a masked display string for a saved API key.
///
/// Shows the first 7 characters, a run of bullet characters, and the last 4
/// characters. For example, `sk-ant-api03-abcdef123456789` becomes
/// `sk-ant-\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}6789`.
///
/// Returns an empty string if the key is empty or shorter than 8 characters
/// (too short to mask meaningfully).
pub fn mask_api_key(key: &str) -> String {
    let char_count = key.chars().count();
    if char_count < 8 {
        // Too short to mask meaningfully; treat as no key
        return String::new();
    }
    let prefix: String = key.chars().take(7).collect();
    let suffix: String = key.chars().skip(char_count - 4).collect();
    let dots = "\u{2022}".repeat(5); // 5 bullet characters
    format!("{}{}{}", prefix, dots, suffix)
}

// ---------------------------------------------------------------------------
// LlmSetupSection
// ---------------------------------------------------------------------------

/// Which section of the LLM setup screen currently has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LlmSetupSection {
    Provider,
    Model,
    ApiKey,
}

impl LlmSetupSection {
    /// Ordered list of sections for the progressive flow.
    pub const CYCLE: &[LlmSetupSection] = &[
        LlmSetupSection::Provider,
        LlmSetupSection::Model,
        LlmSetupSection::ApiKey,
    ];

    /// Advance to the next section (wraps around).
    pub fn next(self) -> LlmSetupSection {
        let idx = Self::CYCLE.iter().position(|&s| s == self).unwrap_or(0);
        Self::CYCLE[(idx + 1) % Self::CYCLE.len()]
    }

    /// Go to the previous section (wraps around).
    pub fn prev(self) -> LlmSetupSection {
        let idx = Self::CYCLE.iter().position(|&s| s == self).unwrap_or(0);
        if idx == 0 {
            Self::CYCLE[Self::CYCLE.len() - 1]
        } else {
            Self::CYCLE[idx - 1]
        }
    }

    /// Return the step index (0-based) of this section.
    pub fn step_index(self) -> usize {
        Self::CYCLE.iter().position(|&s| s == self).unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// LlmConnectionStatus
// ---------------------------------------------------------------------------

/// Status of the API connection test.
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(Default)]
pub enum LlmConnectionStatus {
    /// No test has been run yet.
    #[default]
    Untested,
    /// A test is currently in progress.
    Testing,
    /// The test succeeded.
    Success(String),
    /// The test failed with an error message.
    Failed(String),
}


// ---------------------------------------------------------------------------
// LlmSetupState
// ---------------------------------------------------------------------------

/// UI state for the LLM setup screen.
///
/// Lives inside `ViewState` so the TUI can render it without any global state.
///
/// Note: custom `Debug` implementation redacts `api_key_input` and
/// `api_key_backup` to avoid leaking secrets in log output.
#[derive(Clone)]
pub struct LlmSetupState {
    /// Which section currently has keyboard focus.
    pub active_section: LlmSetupSection,
    /// How far the user has confirmed in the progressive disclosure flow.
    /// `None` means nothing confirmed yet (only Provider is visible).
    /// `Some(Provider)` means provider is confirmed, model is now visible.
    /// `Some(Model)` means model is confirmed, API key is now visible.
    /// `Some(ApiKey)` means API key is confirmed and connection test has run.
    pub confirmed_through: Option<LlmSetupSection>,
    /// Index into the provider list (Anthropic=0, Google=1, OpenAI=2).
    pub selected_provider_idx: usize,
    /// Index into the model list for the currently selected provider.
    pub selected_model_idx: usize,
    /// The API key text as entered by the user (with cursor tracking).
    pub api_key_input: TextInput,
    /// Backup of the API key before entering edit mode, restored on Esc cancel.
    pub api_key_backup: String,
    /// Whether the API key text input is in edit mode.
    pub api_key_editing: bool,
    /// Result of the last connection test.
    pub connection_status: LlmConnectionStatus,
    /// Whether a saved API key exists in credentials (even though the text
    /// input may be empty). Set when entering Settings mode so the UI can
    /// show a masked placeholder instead of a blank field.
    pub has_saved_api_key: bool,
    /// Masked display string for a saved API key (e.g. `sk-ant-***XXXX`).
    /// Populated when entering Settings mode; empty when no saved key exists.
    pub saved_api_key_mask: String,

    // --- Settings mode fields ---

    /// In settings mode, which field is currently open for editing.
    /// `None` means we are in overview mode (all fields shown as summaries).
    /// `Some(section)` means that field's dropdown/editor is open.
    pub settings_editing_field: Option<LlmSetupSection>,
    /// Snapshot of provider index before entering settings edit mode.
    /// Used to restore on Escape.
    pub settings_saved_provider_idx: usize,
    /// Snapshot of model index before entering settings edit mode.
    /// Used to restore on Escape.
    pub settings_saved_model_idx: usize,
    /// Snapshot of API key text before entering settings edit mode.
    /// Used to restore on Escape.
    pub settings_saved_api_key: String,
    /// Whether the user has unsaved changes in settings mode.
    pub settings_dirty: bool,
    /// Snapshot of `confirmed_through` before entering settings edit mode.
    /// Used to restore on Escape.
    pub settings_saved_confirmed_through: Option<LlmSetupSection>,
    /// Explicit flag: true only when the app is in Settings mode (not onboarding).
    /// Set by `ModeChanged` handler; cleared when leaving settings.
    pub in_settings_mode: bool,
    /// Whether any LLM config field (provider, model, or API key) was changed
    /// in settings mode and a connection test is required before saving. Set
    /// when a field is confirmed with a value different from the saved snapshot;
    /// cleared when the connection test succeeds or on Esc (restore snapshot).
    /// While `true`, the 's' (save) keybind is blocked.
    pub settings_needs_connection_test: bool,
    /// Snapshot of `has_saved_api_key` for restore on Escape.
    pub settings_saved_has_api_key: bool,
    /// Snapshot of `saved_api_key_mask` for restore on Escape.
    pub settings_saved_api_key_mask: String,
    /// Stable base ID used to derive state-dependent subscription IDs.
    /// The actual ID is hashed from this plus relevant state fields so the
    /// listener is rebuilt when the active mode/state changes.
    sub_id: SubscriptionId,
}

impl std::fmt::Debug for LlmSetupState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmSetupState")
            .field("active_section", &self.active_section)
            .field("confirmed_through", &self.confirmed_through)
            .field("selected_provider_idx", &self.selected_provider_idx)
            .field("selected_model_idx", &self.selected_model_idx)
            .field("api_key_input", &if self.api_key_input.is_empty() { "(empty)" } else { "[REDACTED]" })
            .field("api_key_backup", &if self.api_key_backup.is_empty() { "(empty)" } else { "[REDACTED]" })
            .field("api_key_editing", &self.api_key_editing)
            .field("connection_status", &self.connection_status)
            .field("has_saved_api_key", &self.has_saved_api_key)
            .field("saved_api_key_mask", &if self.saved_api_key_mask.is_empty() { "(empty)" } else { "[REDACTED]" })
            .field("settings_editing_field", &self.settings_editing_field)
            .field("settings_dirty", &self.settings_dirty)
            .field("settings_needs_connection_test", &self.settings_needs_connection_test)
            .finish()
    }
}

impl Default for LlmSetupState {
    fn default() -> Self {
        LlmSetupState {
            active_section: LlmSetupSection::Provider,
            confirmed_through: None,
            selected_provider_idx: 0,
            selected_model_idx: 0,
            api_key_input: TextInput::new(),
            api_key_backup: String::new(),
            api_key_editing: false,
            connection_status: LlmConnectionStatus::Untested,
            has_saved_api_key: false,
            saved_api_key_mask: String::new(),
            settings_editing_field: None,
            settings_saved_provider_idx: 0,
            settings_saved_model_idx: 0,
            settings_saved_api_key: String::new(),
            settings_dirty: false,
            settings_saved_confirmed_through: None,
            in_settings_mode: false,
            settings_needs_connection_test: false,
            settings_saved_has_api_key: false,
            settings_saved_api_key_mask: String::new(),
            sub_id: SubscriptionId::unique(),
        }
    }
}

impl LlmSetupState {
    /// The ordered list of providers for selection.
    pub const PROVIDERS: &[LlmProvider] = &[
        LlmProvider::Anthropic,
        LlmProvider::Google,
        LlmProvider::OpenAI,
    ];

    /// Return the currently selected provider.
    pub fn selected_provider(&self) -> &LlmProvider {
        &Self::PROVIDERS[self.selected_provider_idx]
    }

    /// Return the models available for the currently selected provider.
    pub fn available_models(&self) -> Vec<&'static ModelOption> {
        models_for_provider(self.selected_provider())
    }

    /// Return the currently selected model, if any.
    pub fn selected_model(&self) -> Option<&'static ModelOption> {
        let models = self.available_models();
        models.get(self.selected_model_idx).copied()
    }

    /// Move provider selection up. Invalidates downstream state if provider changes.
    pub fn provider_up(&mut self) {
        if self.selected_provider_idx > 0 {
            self.selected_provider_idx -= 1;
            self.invalidate_past_provider();
        }
    }

    /// Move provider selection down. Invalidates downstream state if provider changes.
    pub fn provider_down(&mut self) {
        if self.selected_provider_idx + 1 < Self::PROVIDERS.len() {
            self.selected_provider_idx += 1;
            self.invalidate_past_provider();
        }
    }

    /// Move model selection up. Invalidates downstream confirmations if model changes.
    pub fn model_up(&mut self) {
        if self.selected_model_idx > 0 {
            self.selected_model_idx -= 1;
            self.invalidate_past_model();
        }
    }

    /// Move model selection down. Invalidates downstream confirmations if model changes.
    pub fn model_down(&mut self) {
        let count = self.available_models().len();
        if self.selected_model_idx + 1 < count {
            self.selected_model_idx += 1;
            self.invalidate_past_model();
        }
    }

    /// If the user has confirmed past Provider (i.e. Model or ApiKey is confirmed),
    /// reset confirmed_through to at most Provider, reset model selection,
    /// and clear connection status, since the provider just changed.
    ///
    /// Skipped in settings mode (field-editing) to avoid corrupting navigation state.
    fn invalidate_past_provider(&mut self) {
        if self.settings_editing_field.is_some() {
            return;
        }
        if self.confirmed_through > Some(LlmSetupSection::Provider) {
            self.selected_model_idx = 0;
            self.connection_status = LlmConnectionStatus::Untested;
            self.confirmed_through = Some(LlmSetupSection::Provider);
        }
    }

    /// If the user has confirmed past Model (i.e. ApiKey is confirmed),
    /// reset confirmed_through to Provider and clear connection status,
    /// since the model just changed.
    ///
    /// Skipped in settings mode (field-editing) to avoid corrupting navigation state.
    fn invalidate_past_model(&mut self) {
        if self.settings_editing_field.is_some() {
            return;
        }
        if self.confirmed_through > Some(LlmSetupSection::Model) {
            self.confirmed_through = Some(LlmSetupSection::Provider);
            self.connection_status = LlmConnectionStatus::Untested;
        }
    }

    /// Whether a given section is visible in the progressive disclosure flow.
    ///
    /// Provider is always visible. Each subsequent section is visible only if
    /// the previous section has been confirmed.
    pub fn is_section_visible(&self, section: LlmSetupSection) -> bool {
        match section {
            LlmSetupSection::Provider => true,
            LlmSetupSection::Model => {
                matches!(self.confirmed_through, Some(s) if s >= LlmSetupSection::Provider)
            }
            LlmSetupSection::ApiKey => {
                matches!(self.confirmed_through, Some(s) if s >= LlmSetupSection::Model)
            }
        }
    }

    /// Whether a given section has been confirmed (locked in).
    pub fn is_section_confirmed(&self, section: LlmSetupSection) -> bool {
        matches!(self.confirmed_through, Some(s) if s >= section)
    }

    /// Whether the connection test has passed successfully.
    pub fn connection_tested_ok(&self) -> bool {
        matches!(self.connection_status, LlmConnectionStatus::Success(_))
    }

    /// Confirm the current section and advance focus to the next.
    ///
    /// Returns `true` if a new section was revealed.
    pub fn confirm_current_section(&mut self) -> bool {
        let current = self.active_section;
        let current_idx = current.step_index();

        // Update confirmed_through to at least the current section
        let should_update = match self.confirmed_through {
            None => true,
            Some(s) => current > s,
        };
        if should_update {
            self.confirmed_through = Some(current);
        }

        // Advance focus to the next section if there is one
        let sections = LlmSetupSection::CYCLE;
        if current_idx + 1 < sections.len() {
            self.active_section = sections[current_idx + 1];
            // Auto-focus API key text input when reaching ApiKey step
            if self.active_section == LlmSetupSection::ApiKey {
                self.api_key_backup = self.api_key_input.value().to_string();
                self.api_key_editing = true;
            }
            true
        } else {
            false
        }
    }

    /// Go back to the previous section, un-confirming the current one
    /// and clearing all downstream state.
    ///
    /// Returns `true` if the step changed.
    pub fn go_back_section(&mut self) -> bool {
        let current = self.active_section;
        let current_idx = current.step_index();

        if current_idx == 0 {
            return false;
        }

        let prev_section = LlmSetupSection::CYCLE[current_idx - 1];
        self.active_section = prev_section;

        // Un-confirm: set confirmed_through to the section before the one
        // we're now focused on (so the current focus section is "active" again)
        if current_idx >= 2 {
            self.confirmed_through = Some(LlmSetupSection::CYCLE[current_idx - 2]);
        } else {
            self.confirmed_through = None;
        }

        // Clear downstream state depending on where we're going back from
        match current {
            LlmSetupSection::Model => {
                // Going back from Model to Provider: reset model selection
                self.selected_model_idx = 0;
                self.connection_status = LlmConnectionStatus::Untested;
            }
            LlmSetupSection::ApiKey => {
                // Going back from ApiKey to Model: preserve the API key,
                // only clear editing state and connection status
                self.api_key_editing = false;
                self.connection_status = LlmConnectionStatus::Untested;
            }
            LlmSetupSection::Provider => {
                // Can't go back from Provider (handled above), but for completeness
            }
        }

        true
    }

    /// Snapshot the current settings state so it can be restored on Escape.
    pub fn snapshot_settings(&mut self) {
        self.settings_saved_provider_idx = self.selected_provider_idx;
        self.settings_saved_model_idx = self.selected_model_idx;
        self.settings_saved_api_key = self.api_key_input.value().to_string();
        self.settings_saved_confirmed_through = self.confirmed_through;
        self.settings_saved_has_api_key = self.has_saved_api_key;
        self.settings_saved_api_key_mask = self.saved_api_key_mask.clone();
    }

    /// Restore settings to the last saved snapshot (called on Escape).
    pub fn restore_settings_snapshot(&mut self) {
        self.selected_provider_idx = self.settings_saved_provider_idx;
        self.selected_model_idx = self.settings_saved_model_idx;
        self.api_key_input.set_value(&self.settings_saved_api_key.clone());
        self.confirmed_through = self.settings_saved_confirmed_through;
        self.api_key_editing = false;
        self.settings_editing_field = None;
        self.settings_dirty = false;
        self.settings_needs_connection_test = false;
        self.connection_status = LlmConnectionStatus::Untested;
        self.has_saved_api_key = self.settings_saved_has_api_key;
        self.saved_api_key_mask = self.settings_saved_api_key_mask.clone();
    }

    /// Whether the settings page is in field-editing mode (a dropdown/editor is open).
    pub fn is_settings_field_editing(&self) -> bool {
        self.settings_editing_field.is_some()
    }

    /// Whether any LLM config field differs from the saved snapshot.
    pub fn has_config_changed_from_snapshot(&self) -> bool {
        self.selected_provider_idx != self.settings_saved_provider_idx
            || self.selected_model_idx != self.settings_saved_model_idx
            || self.api_key_input.value() != self.settings_saved_api_key
    }

    /// Whether saving is blocked because an LLM config field (provider, model,
    /// or API key) was changed and the connection test has not yet passed.
    pub fn is_save_blocked(&self) -> bool {
        self.settings_needs_connection_test && !self.connection_tested_ok()
    }

    /// Return the API key display text: masked when not editing, raw when editing.
    ///
    /// When the text input is empty but a saved key exists (indicated by
    /// `has_saved_api_key`), returns the pre-computed `saved_api_key_mask`
    /// so the UI shows a placeholder like `sk-ant-***XXXX` instead of blank.
    pub fn api_key_display(&self) -> String {
        let raw = self.api_key_input.value();
        if raw.is_empty() {
            if !self.api_key_editing && self.has_saved_api_key && !self.saved_api_key_mask.is_empty() {
                return self.saved_api_key_mask.clone();
            }
            return String::new();
        }
        if self.api_key_editing {
            raw.to_string()
        } else {
            // Show first 7 chars, then mask the rest
            let visible = raw.chars().take(7).collect::<String>();
            let mask_len = raw.chars().count().saturating_sub(7);
            format!("{}{}", visible, "*".repeat(mask_len))
        }
    }
}

// ---------------------------------------------------------------------------
// ELM message API
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum LlmSetupMessage {
    // Navigation within sections
    SectionUp,
    SectionDown,
    ConfirmSection,
    GoBackSection,
    // API key editing
    StartApiKeyEdit,
    ApiKeyInput(KeyEvent),
    FinishApiKeyEdit,
    CancelApiKeyEdit,
    // Connection / flow
    TestConnection,
    Quit,
    // Settings-mode field navigation
    SettingsFieldUp,
    SettingsFieldDown,
    SettingsFieldOpen,
    SettingsFieldConfirm,
    SettingsFieldCancel,
    SettingsSave,
    // Settings-mode tab switching
    SettingsTabSwitch(SettingsSection),
    // Settings-mode Esc (needs to check cross-component dirty state, so
    // the caller provides the combined dirty flag)
    SettingsExit,
}

impl LlmSetupState {
    /// Declare keybindings for the subscription system.
    ///
    /// State-dependent subscription ID: the listener is rebuilt whenever
    /// `settings_mode`, `api_key_editing`, or `settings_editing_field` changes,
    /// so the correct binding set is always active.
    ///
    /// States (mutually exclusive, highest priority first):
    /// - `api_key_editing` → CAPTURE mode: Enter, Esc, Any key → ApiKeyInput
    /// - settings mode + field editing → Up/k, Down/j, Enter, Esc, q
    /// - settings mode + overview → Up/k, Down/j, Enter, Esc, s, 1, 2, q
    /// - onboarding mode → Up/k, Down/j, Enter, Esc, n, s, Tab, BackTab, q
    pub fn subscription(
        &self,
        kb: &mut KeybindManager,
        settings_mode: bool,
    ) -> Subscription<LlmSetupMessage> {
        // Derive state-dependent ID so the listener is rebuilt on state changes.
        let mut hasher = DefaultHasher::new();
        self.sub_id.hash(&mut hasher);
        (settings_mode as u8).hash(&mut hasher);
        (self.api_key_editing as u8).hash(&mut hasher);
        (self.settings_editing_field.is_some() as u8).hash(&mut hasher);
        let sub_id = SubscriptionId::from_u64(hasher.finish());

        if self.api_key_editing {
            // CAPTURE mode: Enter/Esc are handled explicitly; everything else
            // is forwarded as ApiKeyInput(key) so the TextInput can process it.
            let recipe = KeyBindingRecipe::new(sub_id)
                .priority(PRIORITY_CAPTURE)
                .capture()
                .bind(
                    exact(KeyCode::Enter),
                    |_| LlmSetupMessage::FinishApiKeyEdit,
                    KeybindHint::new("Enter", "Confirm key"),
                )
                .bind(
                    exact(KeyCode::Esc),
                    |_| LlmSetupMessage::CancelApiKeyEdit,
                    KeybindHint::new("Esc", "Cancel"),
                )
                .bind(
                    KeyTrigger::Any,
                    LlmSetupMessage::ApiKeyInput,
                    None,
                );
            return kb.subscribe(recipe);
        }

        if settings_mode && self.settings_editing_field.is_some() {
            // Settings field editing mode (dropdown open): Up/Down to navigate,
            // Enter to confirm, Esc to cancel, q to quit.
            let recipe = KeyBindingRecipe::new(sub_id)
                .priority(PRIORITY_NORMAL)
                .bind(
                    exact(KeyCode::Up),
                    |_| LlmSetupMessage::SectionUp,
                    KeybindHint::new("↑/k", "Up"),
                )
                .bind(
                    exact(KeyCode::Char('k')),
                    |_| LlmSetupMessage::SectionUp,
                    None,
                )
                .bind(
                    exact(KeyCode::Down),
                    |_| LlmSetupMessage::SectionDown,
                    KeybindHint::new("↓/j", "Down"),
                )
                .bind(
                    exact(KeyCode::Char('j')),
                    |_| LlmSetupMessage::SectionDown,
                    None,
                )
                .bind(
                    exact(KeyCode::Enter),
                    |_| LlmSetupMessage::SettingsFieldConfirm,
                    KeybindHint::new("Enter", "Confirm"),
                )
                .bind(
                    exact(KeyCode::Esc),
                    |_| LlmSetupMessage::SettingsFieldCancel,
                    KeybindHint::new("Esc", "Cancel"),
                )
                .bind(
                    exact(KeyCode::Char('q')),
                    |_| LlmSetupMessage::Quit,
                    KeybindHint::new("q", "Quit"),
                );
            return kb.subscribe(recipe);
        }

        if settings_mode {
            // Settings overview mode: navigate fields, open, save, exit, tab switch.
            let recipe = KeyBindingRecipe::new(sub_id)
                .priority(PRIORITY_NORMAL)
                .bind(
                    exact(KeyCode::Up),
                    |_| LlmSetupMessage::SettingsFieldUp,
                    KeybindHint::new("↑/k", "Up"),
                )
                .bind(
                    exact(KeyCode::Char('k')),
                    |_| LlmSetupMessage::SettingsFieldUp,
                    None,
                )
                .bind(
                    exact(KeyCode::Down),
                    |_| LlmSetupMessage::SettingsFieldDown,
                    KeybindHint::new("↓/j", "Down"),
                )
                .bind(
                    exact(KeyCode::Char('j')),
                    |_| LlmSetupMessage::SettingsFieldDown,
                    None,
                )
                .bind(
                    exact(KeyCode::Enter),
                    |_| LlmSetupMessage::SettingsFieldOpen,
                    KeybindHint::new("Enter", "Edit field"),
                )
                .bind(
                    exact(KeyCode::Esc),
                    |_| LlmSetupMessage::SettingsExit,
                    KeybindHint::new("Esc", "Exit settings"),
                )
                .bind(
                    exact(KeyCode::Char('s')),
                    |_| LlmSetupMessage::SettingsSave,
                    KeybindHint::new("s", "Save"),
                )
                .bind(
                    exact(KeyCode::Char('1')),
                    |_| LlmSetupMessage::SettingsTabSwitch(SettingsSection::LlmConfig),
                    KeybindHint::new("1/2", "Switch tab"),
                )
                .bind(
                    exact(KeyCode::Char('2')),
                    |_| LlmSetupMessage::SettingsTabSwitch(SettingsSection::StrategyConfig),
                    None,
                )
                .bind(
                    exact(KeyCode::Char('q')),
                    |_| LlmSetupMessage::Quit,
                    KeybindHint::new("q", "Quit"),
                );
            return kb.subscribe(recipe);
        }

        // Onboarding mode: navigate sections, confirm, go back, next, skip, tab, quit.
        let recipe = KeyBindingRecipe::new(sub_id)
            .priority(PRIORITY_NORMAL)
            .bind(
                exact(KeyCode::Up),
                |_| LlmSetupMessage::SectionUp,
                KeybindHint::new("↑/k", "Up"),
            )
            .bind(
                exact(KeyCode::Char('k')),
                |_| LlmSetupMessage::SectionUp,
                None,
            )
            .bind(
                exact(KeyCode::Down),
                |_| LlmSetupMessage::SectionDown,
                KeybindHint::new("↓/j", "Down"),
            )
            .bind(
                exact(KeyCode::Char('j')),
                |_| LlmSetupMessage::SectionDown,
                None,
            )
            .bind(
                exact(KeyCode::Enter),
                |_| LlmSetupMessage::ConfirmSection,
                KeybindHint::new("Enter", "Confirm"),
            )
            .bind(
                exact(KeyCode::Esc),
                |_| LlmSetupMessage::GoBackSection,
                KeybindHint::new("Esc", "Back"),
            )
            .bind(
                exact(KeyCode::Char('q')),
                |_| LlmSetupMessage::Quit,
                KeybindHint::new("q", "Quit"),
            );
        kb.subscribe(recipe)
    }

    pub fn update(&mut self, msg: LlmSetupMessage) -> Option<UserCommand> {
        match msg {
            // -- API key editing --
            LlmSetupMessage::FinishApiKeyEdit => {
                if self.in_settings_mode {
                    self.update_settings_finish_api_key()
                } else {
                    self.update_onboarding_finish_api_key()
                }
            }
            LlmSetupMessage::CancelApiKeyEdit => {
                self.api_key_input.set_value(&self.api_key_backup.clone());
                self.api_key_editing = false;
                if self.in_settings_mode {
                    self.restore_settings_snapshot();
                } else {
                    self.go_back_section();
                }
                None
            }
            LlmSetupMessage::ApiKeyInput(key) => {
                if let Some(ti_msg) = TextInput::key_to_message(&key) {
                    self.api_key_input.update(ti_msg);
                }
                if self.in_settings_mode {
                    self.settings_dirty = true;
                }
                None
            }
            LlmSetupMessage::StartApiKeyEdit => {
                self.api_key_backup = self.api_key_input.value().to_string();
                self.api_key_editing = true;
                None
            }

            // -- Section navigation (onboarding + settings field editing) --
            LlmSetupMessage::SectionUp => {
                if self.in_settings_mode {
                    if let Some(editing) = self.settings_editing_field {
                        self.update_settings_dropdown_up(editing);
                    }
                } else {
                    match self.active_section {
                        LlmSetupSection::Provider => {
                            self.provider_up();
                            let provider = self.selected_provider().clone();
                            return Some(UserCommand::OnboardingAction(
                                OnboardingAction::SetProvider(provider),
                            ));
                        }
                        LlmSetupSection::Model => {
                            self.model_up();
                            if let Some(model) = self.selected_model() {
                                return Some(UserCommand::OnboardingAction(
                                    OnboardingAction::SetModel(model.model_id.to_string()),
                                ));
                            }
                        }
                        _ => {}
                    }
                }
                None
            }
            LlmSetupMessage::SectionDown => {
                if self.in_settings_mode {
                    if let Some(editing) = self.settings_editing_field {
                        self.update_settings_dropdown_down(editing);
                    }
                } else {
                    match self.active_section {
                        LlmSetupSection::Provider => {
                            self.provider_down();
                            let provider = self.selected_provider().clone();
                            return Some(UserCommand::OnboardingAction(
                                OnboardingAction::SetProvider(provider),
                            ));
                        }
                        LlmSetupSection::Model => {
                            self.model_down();
                            if let Some(model) = self.selected_model() {
                                return Some(UserCommand::OnboardingAction(
                                    OnboardingAction::SetModel(model.model_id.to_string()),
                                ));
                            }
                        }
                        _ => {}
                    }
                }
                None
            }

            LlmSetupMessage::ConfirmSection => {
                self.update_confirm_section()
            }

            LlmSetupMessage::GoBackSection => {
                if self.in_settings_mode {
                    if self.active_section != LlmSetupSection::Provider {
                        self.active_section = self.active_section.prev();
                    }
                    None
                } else if self.active_section == LlmSetupSection::Provider {
                    Some(UserCommand::OnboardingAction(OnboardingAction::GoBack))
                } else {
                    self.go_back_section();
                    None
                }
            }

            LlmSetupMessage::Quit => Some(UserCommand::Quit),
            LlmSetupMessage::TestConnection => {
                self.confirmed_through = Some(LlmSetupSection::ApiKey);
                self.connection_status = LlmConnectionStatus::Testing;
                Some(UserCommand::OnboardingAction(OnboardingAction::TestConnection))
            }

            // -- Settings field navigation --
            LlmSetupMessage::SettingsFieldUp => {
                let idx = self.active_section.step_index();
                if idx > 0 {
                    self.active_section = LlmSetupSection::CYCLE[idx - 1];
                }
                None
            }
            LlmSetupMessage::SettingsFieldDown => {
                let idx = self.active_section.step_index();
                if idx + 1 < LlmSetupSection::CYCLE.len() {
                    self.active_section = LlmSetupSection::CYCLE[idx + 1];
                }
                None
            }
            LlmSetupMessage::SettingsFieldOpen => {
                self.snapshot_settings();
                self.settings_editing_field = Some(self.active_section);
                if self.active_section == LlmSetupSection::ApiKey {
                    self.api_key_backup = self.api_key_input.value().to_string();
                    self.api_key_editing = true;
                }
                None
            }
            LlmSetupMessage::SettingsFieldConfirm => {
                self.update_settings_field_confirm()
            }
            LlmSetupMessage::SettingsFieldCancel => {
                self.restore_settings_snapshot();
                None
            }
            LlmSetupMessage::SettingsSave => {
                if self.is_save_blocked() {
                    None
                } else {
                    let provider = self.selected_provider().clone();
                    let model_id = self
                        .selected_model()
                        .map(|m| m.model_id.to_string())
                        .unwrap_or_default();
                    let api_key_val = self.api_key_input.value().to_string();
                    let api_key = if api_key_val.is_empty() {
                        None
                    } else {
                        Some(api_key_val)
                    };
                    self.settings_dirty = false;
                    self.settings_needs_connection_test = false;
                    self.snapshot_settings();
                    Some(UserCommand::OnboardingAction(
                        OnboardingAction::SaveLlmConfig {
                            provider,
                            model_id,
                            api_key,
                        },
                    ))
                }
            }
            LlmSetupMessage::SettingsTabSwitch(section) => {
                Some(UserCommand::SwitchSettingsTab(section))
            }
            LlmSetupMessage::SettingsExit => {
                // Caller must check cross-component dirty state before sending
                // this message. If this message arrives, it means exit is safe.
                Some(UserCommand::ExitSettings)
            }
        }
    }

    fn update_onboarding_finish_api_key(&mut self) -> Option<UserCommand> {
        self.api_key_editing = false;
        let key = self.api_key_input.value().to_string();
        if key.is_empty() {
            None
        } else {
            self.confirmed_through = Some(LlmSetupSection::ApiKey);
            self.connection_status = LlmConnectionStatus::Testing;
            Some(UserCommand::OnboardingAction(OnboardingAction::SetApiKey(key)))
        }
    }

    fn update_settings_finish_api_key(&mut self) -> Option<UserCommand> {
        let new_key = self.api_key_input.value().to_string();
        let has_key = !new_key.is_empty() || self.has_saved_api_key;
        // If provider changed in cascade and no key available, stay in editing
        let provider_changed = self.selected_provider_idx != self.settings_saved_provider_idx;
        if provider_changed && !has_key {
            return None;
        }
        self.api_key_editing = false;
        self.settings_dirty = true;
        self.settings_editing_field = None;
        self.active_section = LlmSetupSection::ApiKey;
        if self.has_config_changed_from_snapshot() && has_key {
            self.settings_needs_connection_test = true;
            self.connection_status = LlmConnectionStatus::Testing;
            if provider_changed {
                // Provider changed — use TestConnectionWith to avoid mutating backend state
                let provider = self.selected_provider().clone();
                let model_id = self
                    .selected_model()
                    .map(|m| m.model_id.to_string())
                    .unwrap_or_default();
                Some(UserCommand::OnboardingAction(
                    OnboardingAction::TestConnectionWith {
                        provider,
                        model_id,
                        api_key: new_key,
                    },
                ))
            } else if !new_key.is_empty() {
                Some(UserCommand::OnboardingAction(
                    OnboardingAction::SetApiKey(new_key),
                ))
            } else {
                Some(UserCommand::OnboardingAction(
                    OnboardingAction::TestConnection,
                ))
            }
        } else {
            self.settings_needs_connection_test = false;
            self.connection_status = LlmConnectionStatus::Untested;
            None
        }
    }

    fn update_settings_dropdown_up(&mut self, editing: LlmSetupSection) {
        match editing {
            LlmSetupSection::Provider => {
                let before = self.selected_provider_idx;
                self.provider_up();
                if self.selected_provider_idx != before {
                    self.settings_dirty = true;
                }
            }
            LlmSetupSection::Model => {
                let before = self.selected_model_idx;
                self.model_up();
                if self.selected_model_idx != before {
                    self.settings_dirty = true;
                }
            }
            LlmSetupSection::ApiKey => {}
        }
    }

    fn update_settings_dropdown_down(&mut self, editing: LlmSetupSection) {
        match editing {
            LlmSetupSection::Provider => {
                let before = self.selected_provider_idx;
                self.provider_down();
                if self.selected_provider_idx != before {
                    self.settings_dirty = true;
                }
            }
            LlmSetupSection::Model => {
                let before = self.selected_model_idx;
                self.model_down();
                if self.selected_model_idx != before {
                    self.settings_dirty = true;
                }
            }
            LlmSetupSection::ApiKey => {}
        }
    }

    fn update_settings_field_confirm(&mut self) -> Option<UserCommand> {
        let editing = self.settings_editing_field?;
        match editing {
            LlmSetupSection::Provider => {
                if self.in_settings_mode {
                    let provider_changed = self.selected_provider_idx != self.settings_saved_provider_idx;
                    if provider_changed {
                        // Provider changed: reset model for new provider and cascade to model selection
                        self.selected_model_idx = 0;
                        self.settings_dirty = true;
                        self.settings_editing_field = Some(LlmSetupSection::Model);
                        self.active_section = LlmSetupSection::Model;
                        None
                    } else {
                        // No change, just close
                        self.settings_editing_field = None;
                        None
                    }
                } else {
                    // Onboarding: advance to next field in sequence
                    self.active_section = LlmSetupSection::Model;
                    self.settings_editing_field = Some(LlmSetupSection::Model);
                    None
                }
            }
            LlmSetupSection::Model => {
                if self.in_settings_mode {
                    let provider_changed = self.selected_provider_idx != self.settings_saved_provider_idx;
                    if provider_changed {
                        // Provider was changed earlier in cascade, continue to API key
                        self.settings_dirty = true;
                        self.settings_editing_field = Some(LlmSetupSection::ApiKey);
                        self.active_section = LlmSetupSection::ApiKey;
                        // Clear API key since provider changed (different provider = different key)
                        self.api_key_input.set_value("");
                        self.has_saved_api_key = false;
                        self.saved_api_key_mask = String::new();
                        self.api_key_backup = String::new();
                        self.api_key_editing = true;
                        None
                    } else {
                        // Only model changed, close dropdown
                        self.settings_editing_field = None;
                        let has_key = !self.api_key_input.value().is_empty()
                            || self.has_saved_api_key;
                        if self.has_config_changed_from_snapshot() && has_key {
                            self.settings_needs_connection_test = true;
                            self.connection_status = LlmConnectionStatus::Testing;
                            let new_key = self.api_key_input.value().to_string();
                            if !new_key.is_empty() {
                                return Some(UserCommand::OnboardingAction(
                                    OnboardingAction::SetApiKey(new_key),
                                ));
                            } else {
                                return Some(UserCommand::OnboardingAction(
                                    OnboardingAction::TestConnection,
                                ));
                            }
                        }
                        None
                    }
                } else {
                    // Onboarding: advance to API key
                    self.active_section = LlmSetupSection::ApiKey;
                    self.settings_editing_field = Some(LlmSetupSection::ApiKey);
                    self.api_key_backup = self.api_key_input.value().to_string();
                    self.api_key_editing = true;
                    None
                }
            }
            LlmSetupSection::ApiKey => {
                self.settings_editing_field = None;
                None
            }
        }
    }

    fn update_confirm_section(&mut self) -> Option<UserCommand> {
        match self.active_section {
            LlmSetupSection::Provider => {
                let provider = self.selected_provider().clone();
                self.confirm_current_section();
                Some(UserCommand::OnboardingAction(
                    OnboardingAction::SetProvider(provider),
                ))
            }
            LlmSetupSection::Model => {
                let model_id = self
                    .selected_model()
                    .map(|m| m.model_id.to_string())
                    .unwrap_or_default();
                self.confirm_current_section();
                Some(UserCommand::OnboardingAction(
                    OnboardingAction::SetModel(model_id),
                ))
            }
            LlmSetupSection::ApiKey => {
                if self.connection_tested_ok() {
                    Some(UserCommand::OnboardingAction(OnboardingAction::GoNext))
                } else if self.api_key_input.is_empty() {
                    self.api_key_backup = self.api_key_input.value().to_string();
                    self.api_key_editing = true;
                    None
                } else if matches!(self.connection_status, LlmConnectionStatus::Failed(_)) {
                    self.api_key_backup = self.api_key_input.value().to_string();
                    self.api_key_editing = true;
                    self.connection_status = LlmConnectionStatus::Untested;
                    None
                } else {
                    self.confirmed_through = Some(LlmSetupSection::ApiKey);
                    self.connection_status = LlmConnectionStatus::Testing;
                    Some(UserCommand::OnboardingAction(
                        OnboardingAction::TestConnection,
                    ))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

/// Render the LLM setup screen into the given area.
///
/// Uses progressive disclosure: sections appear one at a time as each is
/// confirmed. Confirmed sections display in a compact "locked" state.
pub fn render(frame: &mut Frame, area: Rect, state: &LlmSetupState) {
    // Outer block with title
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Line::from(vec![Span::styled(
            " Configure Your AI Assistant ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]))
        .title_alignment(Alignment::Center);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Determine which sections are visible / confirmed
    let provider_visible = true; // always
    let provider_confirmed = state.is_section_confirmed(LlmSetupSection::Provider);
    let model_visible = state.is_section_visible(LlmSetupSection::Model);
    let model_confirmed = state.is_section_confirmed(LlmSetupSection::Model);
    let apikey_visible = state.is_section_visible(LlmSetupSection::ApiKey);

    // In settings mode, determine which fields should be shown expanded vs compact.
    // - Overview mode (settings_editing_field == None): all compact, active highlighted
    // - Field editing mode: only the editing field is expanded
    let is_settings_mode = state.in_settings_mode
        && ((state.confirmed_through == Some(LlmSetupSection::ApiKey)
            && !state.api_key_editing
            && state.settings_editing_field.is_none())
            || state.settings_editing_field.is_some());

    // A section should be shown expanded (full list) if:
    // - In onboarding: it's the active section and not yet confirmed past it
    // - In settings field editing: it matches settings_editing_field
    // - In settings overview: never (all compact)
    let provider_expanded = if state.settings_editing_field.is_some() {
        state.settings_editing_field == Some(LlmSetupSection::Provider)
    } else if is_settings_mode {
        false // overview mode: all compact
    } else {
        // Onboarding: original logic
        !(provider_confirmed && state.active_section != LlmSetupSection::Provider)
    };

    let model_expanded = if state.settings_editing_field.is_some() {
        state.settings_editing_field == Some(LlmSetupSection::Model)
    } else if is_settings_mode {
        false
    } else {
        model_visible && !(model_confirmed && state.active_section != LlmSetupSection::Model)
    };

    let apikey_expanded = if state.settings_editing_field.is_some() {
        state.settings_editing_field == Some(LlmSetupSection::ApiKey)
    } else if is_settings_mode {
        false
    } else {
        apikey_visible
    };

    // Build dynamic layout constraints based on visibility
    let mut constraints: Vec<Constraint> = Vec::new();
    constraints.push(Constraint::Length(1)); // [0] top padding

    // Provider section (always visible)
    if !provider_expanded {
        // Compact: label + confirmed value on one line
        constraints.push(Constraint::Length(2)); // [1] "Provider: <value>"
    } else {
        constraints.push(Constraint::Length(1)); // [1] "Provider:" label
        constraints.push(Constraint::Length(LlmSetupState::PROVIDERS.len() as u16 + 1)); // [2] provider list
    }

    // Model section (visible after provider confirmed)
    if model_visible {
        if !model_expanded {
            constraints.push(Constraint::Length(2)); // compact
        } else {
            let models = state.available_models();
            constraints.push(Constraint::Length(1)); // "Model:" label
            constraints.push(Constraint::Length(models.len().min(4) as u16 + 1)); // model list
        }
    }

    // API Key section (visible after model confirmed)
    if apikey_visible {
        if !apikey_expanded && !state.api_key_editing {
            // Compact display for API key in settings overview
            constraints.push(Constraint::Length(2)); // compact
            // Inline connection status in settings overview (not Untested)
            if is_settings_mode
                && !matches!(state.connection_status, LlmConnectionStatus::Untested)
            {
                constraints.push(Constraint::Length(1)); // status line
            }
        } else {
            constraints.push(Constraint::Length(1)); // "API Key:" label
            constraints.push(Constraint::Length(2)); // api key input + spacing
            // Connection status (always show if api key section is visible and test has been run)
            if !matches!(state.connection_status, LlmConnectionStatus::Untested) {
                constraints.push(Constraint::Length(2)); // connection status
            }
            // "Press Enter to continue..." prompt (only after successful test)
            if matches!(state.connection_status, LlmConnectionStatus::Success(_)) {
                constraints.push(Constraint::Length(2)); // continue prompt
            }
        }
    }

    constraints.push(Constraint::Min(0)); // flexible space

    let sections = Layout::vertical(constraints).split(inner);

    // Horizontal centering — use most of the available width so long
    // API keys are not truncated, but cap at 80 for readability on very
    // wide terminals.  Leave at least 2 columns of padding on each side.
    let content_width = 80u16.min(inner.width.saturating_sub(4));
    let h_offset = (inner.width.saturating_sub(content_width)) / 2;
    let content_rect = |row: Rect| -> Rect {
        Rect {
            x: row.x + h_offset,
            y: row.y,
            width: content_width.min(row.width.saturating_sub(h_offset)),
            height: row.height,
        }
    };

    let mut slot = 1usize; // current layout slot index (0 is top padding)

    // ---- Provider section ----
    if provider_visible {
        let provider_active = state.active_section == LlmSetupSection::Provider;
        if !provider_expanded {
            // Compact display
            if provider_active && is_settings_mode {
                // Highlighted compact line in settings overview
                render_highlighted_line(
                    frame,
                    content_rect(sections[slot]),
                    "Provider",
                    state.selected_provider().display_name(),
                );
            } else {
                render_confirmed_line(
                    frame,
                    content_rect(sections[slot]),
                    "Provider",
                    state.selected_provider().display_name(),
                );
            }
            slot += 1;
        } else {
            // Full interactive list
            let label_style = if provider_active {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let label = Paragraph::new(Line::from(vec![Span::styled(
                "Provider:",
                label_style,
            )]));
            frame.render_widget(label, content_rect(sections[slot]));
            slot += 1;

            let list_area = content_rect(sections[slot]);
            for (i, provider) in LlmSetupState::PROVIDERS.iter().enumerate() {
                let is_selected = i == state.selected_provider_idx;
                let prefix = if is_selected { "> " } else { "  " };
                let style = if is_selected && provider_active {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else if is_selected {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                };
                let line = Paragraph::new(Line::from(vec![Span::styled(
                    format!("{}{}", prefix, provider.display_name()),
                    style,
                )]));
                let row_rect = Rect {
                    x: list_area.x,
                    y: list_area.y + i as u16,
                    width: list_area.width,
                    height: 1,
                };
                frame.render_widget(line, row_rect);
            }
            slot += 1;
        }
    }

    // ---- Model section ----
    if model_visible {
        let model_active = state.active_section == LlmSetupSection::Model;
        if !model_expanded {
            // Compact display
            let model_name = state
                .selected_model()
                .map(|m| m.display_name)
                .unwrap_or("(none)");
            if model_active && is_settings_mode {
                render_highlighted_line(frame, content_rect(sections[slot]), "Model", model_name);
            } else {
                render_confirmed_line(frame, content_rect(sections[slot]), "Model", model_name);
            }
            slot += 1;
        } else {
            // Full interactive list
            let label_style = if model_active {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            let label = Paragraph::new(Line::from(vec![Span::styled("Model:", label_style)]));
            frame.render_widget(label, content_rect(sections[slot]));
            slot += 1;

            let models = state.available_models();
            let list_area = content_rect(sections[slot]);
            for (i, model) in models.iter().enumerate() {
                let is_selected = i == state.selected_model_idx;
                let prefix = if is_selected { "> " } else { "  " };
                let style = if is_selected && model_active {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else if is_selected {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                };

                let tier_style = Style::default().fg(Color::DarkGray);
                let line = Paragraph::new(Line::from(vec![
                    Span::styled(format!("{}{}", prefix, model.display_name), style),
                    Span::styled(format!("  {}", model.tier), tier_style),
                ]));
                let row_rect = Rect {
                    x: list_area.x,
                    y: list_area.y + i as u16,
                    width: list_area.width,
                    height: 1,
                };
                if row_rect.y < list_area.y + list_area.height {
                    frame.render_widget(line, row_rect);
                }
            }
            slot += 1;
        }
    }

    // ---- API Key section ----
    if apikey_visible {
        let key_active = state.active_section == LlmSetupSection::ApiKey;

        // Compact display for settings overview mode
        if !apikey_expanded && !state.api_key_editing {
            let display = state.api_key_display();
            let value = if display.is_empty() {
                "(not set)".to_string()
            } else {
                display
            };
            if key_active && is_settings_mode {
                render_highlighted_line(frame, content_rect(sections[slot]), "API Key", &value);
            } else {
                render_confirmed_line(frame, content_rect(sections[slot]), "API Key", &value);
            }
            slot += 1;

            // Inline connection status in settings overview
            if is_settings_mode
                && !matches!(state.connection_status, LlmConnectionStatus::Untested)
            {
                let status_area = content_rect(sections[slot]);
                let status_line = match &state.connection_status {
                    LlmConnectionStatus::Untested => unreachable!(),
                    LlmConnectionStatus::Testing => Line::from(vec![
                        Span::styled("  Status: ", Style::default().fg(Color::Gray)),
                        Span::styled("Testing...", Style::default().fg(Color::Yellow)),
                    ]),
                    LlmConnectionStatus::Success(msg) => Line::from(vec![
                        Span::styled("  Status: ", Style::default().fg(Color::Gray)),
                        Span::styled("* ", Style::default().fg(Color::Green)),
                        Span::styled(msg.as_str(), Style::default().fg(Color::Green)),
                    ]),
                    LlmConnectionStatus::Failed(msg) => Line::from(vec![
                        Span::styled("  Status: ", Style::default().fg(Color::Gray)),
                        Span::styled("x ", Style::default().fg(Color::Red)),
                        Span::styled(msg.as_str(), Style::default().fg(Color::Red)),
                    ]),
                };
                frame.render_widget(Paragraph::new(status_line), status_area);
                #[allow(unused_assignments)]
                { slot += 1; }
            }
        } else {
        // Expanded display (original rendering)
        let key_label_style = if key_active {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let key_label = Paragraph::new(Line::from(vec![Span::styled(
            "API Key:",
            key_label_style,
        )]));
        frame.render_widget(key_label, content_rect(sections[slot]));
        slot += 1;

        // API key input
        let key_area = content_rect(sections[slot]);
        let display_text = state.api_key_display();
        let is_empty = display_text.is_empty();

        let key_para = if state.api_key_editing {
            let cursor_char = state.api_key_input.cursor_pos();
            let before: String = display_text.chars().take(cursor_char).collect();
            let after: String = display_text.chars().skip(cursor_char).collect();
            let text_style = Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD);
            let cursor_style = Style::default().fg(Color::Black).bg(Color::Cyan);
            Paragraph::new(Line::from(vec![
                Span::styled("  [", Style::default()),
                Span::styled(before, text_style),
                Span::styled("|", cursor_style),
                Span::styled(after, text_style),
                Span::styled("]", Style::default()),
            ]))
        } else {
            let key_text = if is_empty {
                "(press Enter to input)".to_string()
            } else {
                display_text
            };
            let key_style = if is_empty {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::Gray)
            };
            Paragraph::new(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(format!("[{}]", key_text), key_style),
            ]))
        };
        frame.render_widget(key_para, key_area);
        slot += 1;

        // Connection status (only if a test has been run)
        if !matches!(state.connection_status, LlmConnectionStatus::Untested) {
            let status_area = content_rect(sections[slot]);
            let status_line = match &state.connection_status {
                LlmConnectionStatus::Untested => unreachable!(),
                LlmConnectionStatus::Testing => Line::from(vec![
                    Span::styled("  Status: ", Style::default().fg(Color::Gray)),
                    Span::styled("Testing...", Style::default().fg(Color::Yellow)),
                ]),
                LlmConnectionStatus::Success(msg) => Line::from(vec![
                    Span::styled("  Status: ", Style::default().fg(Color::Gray)),
                    Span::styled("* ", Style::default().fg(Color::Green)),
                    Span::styled(msg.as_str(), Style::default().fg(Color::Green)),
                ]),
                LlmConnectionStatus::Failed(msg) => Line::from(vec![
                    Span::styled("  Status: ", Style::default().fg(Color::Gray)),
                    Span::styled("x ", Style::default().fg(Color::Red)),
                    Span::styled(msg.as_str(), Style::default().fg(Color::Red)),
                ]),
            };
            frame.render_widget(Paragraph::new(status_line), status_area);
            slot += 1;
        }

        // "Press Enter to continue..." prompt (only after successful test)
        if matches!(state.connection_status, LlmConnectionStatus::Success(_)) {
            let continue_area = content_rect(sections[slot]);
            let continue_line = Line::from(vec![Span::styled(
                "  Press Enter to continue...",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )]);
            frame.render_widget(Paragraph::new(continue_line), continue_area);
            #[allow(unused_assignments)]
            { slot += 1; }
        }
        } // end expanded API key else block
    }

}

/// Render a single-line confirmed section: "  label: value  [checkmark]"
fn render_confirmed_line(frame: &mut Frame, area: Rect, label: &str, value: &str) {
    let line = Paragraph::new(Line::from(vec![
        Span::styled(
            format!("{}: ", label),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            value.to_string(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" *", Style::default().fg(Color::Green)),
    ]));
    frame.render_widget(line, area);
}

/// Render a single-line highlighted section for settings overview navigation.
/// Uses cyan to indicate the currently focused field: "> label: value"
fn render_highlighted_line(frame: &mut Frame, area: Rect, label: &str, value: &str) {
    let line = Paragraph::new(Line::from(vec![
        Span::styled(
            "> ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}: ", label),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            value.to_string(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    frame.render_widget(line, area);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_selects_anthropic() {
        let state = LlmSetupState::default();
        assert_eq!(*state.selected_provider(), LlmProvider::Anthropic);
        assert_eq!(state.selected_provider_idx, 0);
        assert_eq!(state.selected_model_idx, 0);
        assert!(!state.api_key_editing);
        assert_eq!(state.connection_status, LlmConnectionStatus::Untested);
    }

    #[test]
    fn provider_down_up() {
        let mut state = LlmSetupState::default();
        state.provider_down();
        assert_eq!(*state.selected_provider(), LlmProvider::Google);

        state.provider_down();
        assert_eq!(*state.selected_provider(), LlmProvider::OpenAI);

        // At bottom, stays at bottom
        state.provider_down();
        assert_eq!(*state.selected_provider(), LlmProvider::OpenAI);

        state.provider_up();
        assert_eq!(*state.selected_provider(), LlmProvider::Google);

        state.provider_up();
        assert_eq!(*state.selected_provider(), LlmProvider::Anthropic);

        // At top, stays at top
        state.provider_up();
        assert_eq!(*state.selected_provider(), LlmProvider::Anthropic);
    }

    #[test]
    fn model_selection_resets_on_go_back_from_model() {
        let mut state = LlmSetupState::default();
        // Confirm provider, advance to Model
        state.confirm_current_section();
        // Select second model
        state.model_down();
        assert_eq!(state.selected_model_idx, 1);

        // Go back from Model to Provider resets model selection
        state.go_back_section();
        assert_eq!(state.selected_model_idx, 0);
    }

    #[test]
    fn model_up_down() {
        let mut state = LlmSetupState::default();
        let model_count = state.available_models().len();
        assert!(model_count >= 2);

        state.model_down();
        assert_eq!(state.selected_model_idx, 1);

        // Try to go past end
        for _ in 0..10 {
            state.model_down();
        }
        assert_eq!(state.selected_model_idx, model_count - 1);

        state.model_up();
        assert_eq!(state.selected_model_idx, model_count - 2);

        // Back to top
        for _ in 0..10 {
            state.model_up();
        }
        assert_eq!(state.selected_model_idx, 0);
    }

    #[test]
    fn api_key_display_masked_when_not_editing() {
        let mut state = LlmSetupState::default();
        state.api_key_input.set_value("sk-ant-api03-abcdef123456789");
        state.api_key_editing = false;

        let display = state.api_key_display();
        assert!(display.starts_with("sk-ant-"));
        assert!(display.contains('*'));
        assert_eq!(display.len(), state.api_key_input.value().len());
    }

    #[test]
    fn api_key_display_visible_when_editing() {
        let mut state = LlmSetupState::default();
        state.api_key_input.set_value("sk-ant-api03-abcdef123456789");
        state.api_key_editing = true;

        let display = state.api_key_display();
        assert_eq!(display, state.api_key_input.value());
    }

    #[test]
    fn api_key_display_empty_no_saved_key() {
        let state = LlmSetupState::default();
        assert!(state.api_key_display().is_empty());
    }

    #[test]
    fn section_next_wraps() {
        let s = LlmSetupSection::Provider;
        assert_eq!(s.next(), LlmSetupSection::Model);
        assert_eq!(s.next().next(), LlmSetupSection::ApiKey);
        assert_eq!(s.next().next().next(), LlmSetupSection::Provider); // wraps
    }

    #[test]
    fn section_prev_wraps() {
        let s = LlmSetupSection::Provider;
        assert_eq!(s.prev(), LlmSetupSection::ApiKey); // wraps
        assert_eq!(s.prev().prev(), LlmSetupSection::Model);
    }

    #[test]
    fn selected_model_returns_correct_model() {
        let state = LlmSetupState::default();
        let model = state.selected_model().unwrap();
        assert_eq!(model.provider, LlmProvider::Anthropic);
    }

    #[test]
    fn connection_status_default() {
        assert_eq!(LlmConnectionStatus::default(), LlmConnectionStatus::Untested);
    }

    #[test]
    fn render_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = LlmSetupState::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_with_api_key_editing() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = LlmSetupState::default();
        state.confirmed_through = Some(LlmSetupSection::Model); // make ApiKey visible
        state.api_key_editing = true;
        state.api_key_input.set_value("sk-test");
        state.active_section = LlmSetupSection::ApiKey;
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_with_connection_status_variants() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        for status in [
            LlmConnectionStatus::Untested,
            LlmConnectionStatus::Testing,
            LlmConnectionStatus::Success("Connected!".to_string()),
            LlmConnectionStatus::Failed("Invalid API key".to_string()),
        ] {
            let mut state = LlmSetupState::default();
            state.confirmed_through = Some(LlmSetupSection::Model); // make ApiKey visible
            state.active_section = LlmSetupSection::ApiKey;
            state.connection_status = status;
            terminal
                .draw(|frame| render(frame, frame.area(), &state))
                .unwrap();
        }
    }

    #[test]
    fn progressive_disclosure_visibility() {
        // Initially only Provider is visible
        let state = LlmSetupState::default();
        assert!(state.is_section_visible(LlmSetupSection::Provider));
        assert!(!state.is_section_visible(LlmSetupSection::Model));
        assert!(!state.is_section_visible(LlmSetupSection::ApiKey));

        // After confirming Provider, Model becomes visible
        let mut state = LlmSetupState::default();
        state.confirm_current_section();
        assert!(state.is_section_visible(LlmSetupSection::Provider));
        assert!(state.is_section_visible(LlmSetupSection::Model));
        assert!(!state.is_section_visible(LlmSetupSection::ApiKey));
        assert_eq!(state.active_section, LlmSetupSection::Model);

        // After confirming Model, ApiKey becomes visible
        state.confirm_current_section();
        assert!(state.is_section_visible(LlmSetupSection::Provider));
        assert!(state.is_section_visible(LlmSetupSection::Model));
        assert!(state.is_section_visible(LlmSetupSection::ApiKey));
        assert_eq!(state.active_section, LlmSetupSection::ApiKey);
    }

    #[test]
    fn go_back_section() {
        let mut state = LlmSetupState::default();
        // Confirm through to ApiKey
        state.confirm_current_section(); // Provider -> Model
        state.confirm_current_section(); // Model -> ApiKey

        // Go back to Model
        assert!(state.go_back_section());
        assert_eq!(state.active_section, LlmSetupSection::Model);
        assert_eq!(state.confirmed_through, Some(LlmSetupSection::Provider));

        // Go back to Provider
        assert!(state.go_back_section());
        assert_eq!(state.active_section, LlmSetupSection::Provider);
        assert_eq!(state.confirmed_through, None);

        // Can't go back further
        assert!(!state.go_back_section());
        assert_eq!(state.active_section, LlmSetupSection::Provider);
    }

    #[test]
    fn connection_tested_ok() {
        let mut state = LlmSetupState::default();
        assert!(!state.connection_tested_ok());

        state.connection_status = LlmConnectionStatus::Testing;
        assert!(!state.connection_tested_ok());

        state.connection_status = LlmConnectionStatus::Failed("error".to_string());
        assert!(!state.connection_tested_ok());

        state.connection_status = LlmConnectionStatus::Success("ok".to_string());
        assert!(state.connection_tested_ok());
    }

    #[test]
    fn render_progressive_all_stages() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        // Stage 1: only Provider visible
        let state = LlmSetupState::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();

        // Stage 2: Provider confirmed, Model visible
        let mut state = LlmSetupState::default();
        state.confirm_current_section();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();

        // Stage 3: Model confirmed, ApiKey visible
        let mut state = LlmSetupState::default();
        state.confirm_current_section();
        state.confirm_current_section();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();

        // Stage 4: Connection tested successfully
        let mut state = LlmSetupState::default();
        state.confirm_current_section();
        state.confirm_current_section();
        state.confirmed_through = Some(LlmSetupSection::ApiKey);
        state.connection_status = LlmConnectionStatus::Success("Connected!".to_string());
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    // --- mask_api_key tests ---

    #[test]
    fn mask_api_key_typical_key() {
        let masked = mask_api_key("sk-ant-api03-abcdef123456789");
        // First 7 chars + 5 bullets + last 4 chars
        assert!(masked.starts_with("sk-ant-"));
        assert!(masked.ends_with("6789"));
        assert!(masked.contains('\u{2022}'));
    }

    #[test]
    fn mask_api_key_empty() {
        assert!(mask_api_key("").is_empty());
    }

    #[test]
    fn mask_api_key_too_short() {
        // Keys shorter than 8 chars cannot be meaningfully masked
        assert!(mask_api_key("sk-1234").is_empty());
    }

    #[test]
    fn mask_api_key_exactly_8_chars() {
        let masked = mask_api_key("12345678");
        assert!(masked.starts_with("1234567"));
        assert!(masked.ends_with("5678"));
    }

    // --- api_key_display with saved_api_key_mask ---

    #[test]
    fn api_key_display_shows_saved_mask_when_input_empty() {
        let mut state = LlmSetupState::default();
        state.has_saved_api_key = true;
        state.saved_api_key_mask = "sk-ant-\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}6789".to_string();
        state.api_key_editing = false;

        let display = state.api_key_display();
        assert_eq!(display, state.saved_api_key_mask);
    }

    #[test]
    fn api_key_display_empty_during_editing_even_with_saved_key() {
        let mut state = LlmSetupState::default();
        state.has_saved_api_key = true;
        state.saved_api_key_mask = "sk-ant-\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}6789".to_string();
        state.api_key_editing = true;

        // When editing with empty input, display should be empty (user types from scratch)
        let display = state.api_key_display();
        assert!(display.is_empty());
    }

    #[test]
    fn api_key_display_prefers_typed_input_over_saved_mask() {
        let mut state = LlmSetupState::default();
        state.has_saved_api_key = true;
        state.saved_api_key_mask = "sk-ant-\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}6789".to_string();
        state.api_key_input.set_value("sk-new-key-here");
        state.api_key_editing = false;

        let display = state.api_key_display();
        // Should show the typed key's mask, not the saved mask
        assert!(display.starts_with("sk-new-"));
        assert!(display.contains('*'));
    }

    #[test]
    fn render_with_saved_api_key_mask_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = LlmSetupState::default();
        state.confirmed_through = Some(LlmSetupSection::Model);
        state.active_section = LlmSetupSection::ApiKey;
        state.has_saved_api_key = true;
        state.saved_api_key_mask = "sk-ant-\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}6789".to_string();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }
}
