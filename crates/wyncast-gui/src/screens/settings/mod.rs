// Settings screen — Phase 3.11.
//
// Accessible via `,` from Draft mode. Renders two tabs (LLM Config / Strategy)
// reusing the shared forms/ module. "Save & Close" dispatches SaveAndExitSettings;
// Esc with unsaved changes opens the discard modal.

mod llm_section;
mod strategy_section;
mod unsaved_modal;

use iced::{Element, Length, Padding, Task};
use twui::{
    BoxStyle, ButtonStyle, ButtonVariant, Colors, StackAlign, StackGap, StackStyle, Tab, TabBarStyle,
    TextColor, TextSize, TextStyle, TextWeight, button, frame, h_stack, section_box, tab_bar,
    text, v_stack,
};
use wyncast_app::protocol::{
    OnboardingAction, OnboardingUpdate, SettingsSection, UserCommand,
};

use crate::forms::llm_form::{ConnectionTestState, LlmFormMessage, LlmFormState};
use crate::forms::strategy_form::{StrategyFormMessage, StrategyFormState};

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum SettingsMessage {
    SectionSelected(SettingsSection),
    LlmFormChanged(LlmFormMessage),
    StrategyFormChanged(StrategyFormMessage),
    SaveRequested,
    CancelRequested,
    DiscardConfirmed,
    DiscardCancelled,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct SettingsScreen {
    pub active_section: SettingsSection,
    pub llm: LlmFormState,
    pub strategy: StrategyFormState,
    /// Whether LLM form fields have unsaved changes.
    pub llm_dirty: bool,
    /// Whether strategy form fields have unsaved changes.
    pub strategy_dirty: bool,
    /// Whether the "Discard changes?" modal is open.
    pub discard_modal_open: bool,
}

impl SettingsScreen {
    pub fn new() -> Self {
        Self {
            active_section: SettingsSection::LlmConfig,
            llm: LlmFormState::default(),
            strategy: StrategyFormState::default(),
            llm_dirty: false,
            strategy_dirty: false,
            discard_modal_open: false,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.llm_dirty || self.strategy_dirty
    }

    /// Reset dirty flags and discard modal state (called when settings is opened).
    pub fn reset_dirty(&mut self) {
        self.llm_dirty = false;
        self.strategy_dirty = false;
        self.discard_modal_open = false;
    }

    /// Populate forms from an OnboardingUpdate (ProgressSync carries current config).
    pub fn apply_update(&mut self, update: &OnboardingUpdate) {
        if let OnboardingUpdate::ProgressSync { provider, model, api_key_mask } = update {
            if let Some(p) = provider {
                self.llm.provider = Some(p.clone());
            }
            if let Some(m) = model {
                self.llm.model_id = Some(m.clone());
            }
            self.llm.api_key_mask = api_key_mask.clone();
            // Reset dirty after sync (we just loaded current saved state)
            self.llm_dirty = false;
        }
        if let OnboardingUpdate::ConnectionTestResult { success, message } = update {
            self.llm.connection_test = if *success {
                ConnectionTestState::Success
            } else {
                ConnectionTestState::Failed
            };
            self.llm.connection_test_message = message.clone();
        }
    }

    pub fn update(&mut self, msg: SettingsMessage) -> (Task<SettingsMessage>, Vec<UserCommand>) {
        let mut cmds = Vec::new();
        match msg {
            SettingsMessage::SectionSelected(section) => {
                self.active_section = section;
                cmds.push(UserCommand::SwitchSettingsTab(section));
            }
            SettingsMessage::LlmFormChanged(llm_msg) => {
                let was_test = matches!(llm_msg, LlmFormMessage::TestConnection);
                let dirty = self.llm.apply(llm_msg);
                if dirty {
                    self.llm_dirty = true;
                }
                if was_test {
                    if let (Some(provider), Some(model_id)) =
                        (&self.llm.provider, &self.llm.model_id)
                    {
                        self.llm.connection_test = ConnectionTestState::Testing;
                        let api_key = self.llm.api_key.clone();
                        cmds.push(UserCommand::OnboardingAction(
                            OnboardingAction::TestConnectionWith {
                                provider: provider.clone(),
                                model_id: model_id.clone(),
                                api_key,
                            },
                        ));
                    }
                }
            }
            SettingsMessage::StrategyFormChanged(strategy_msg) => {
                let dirty = self.strategy.apply(strategy_msg);
                if dirty {
                    self.strategy_dirty = true;
                }
            }
            SettingsMessage::SaveRequested => {
                cmds.push(self.build_save_command());
            }
            SettingsMessage::CancelRequested => {
                if self.is_dirty() {
                    self.discard_modal_open = true;
                } else {
                    cmds.push(UserCommand::ExitSettings);
                }
            }
            SettingsMessage::DiscardConfirmed => {
                self.discard_modal_open = false;
                self.reset_dirty();
                cmds.push(UserCommand::ExitSettings);
            }
            SettingsMessage::DiscardCancelled => {
                self.discard_modal_open = false;
            }
        }
        (Task::none(), cmds)
    }

    fn build_save_command(&self) -> UserCommand {
        let llm = if self.llm_dirty {
            if let (Some(provider), Some(model_id)) = (&self.llm.provider, &self.llm.model_id) {
                let api_key = if self.llm.api_key.is_empty() {
                    None
                } else {
                    Some(self.llm.api_key.clone())
                };
                Some((provider.clone(), model_id.clone(), api_key))
            } else {
                None
            }
        } else {
            None
        };

        let strategy = if self.strategy_dirty {
            let overview = if self.strategy.strategy_overview.is_empty() {
                None
            } else {
                Some(self.strategy.strategy_overview.clone())
            };
            Some((
                self.strategy.hitting_budget_pct,
                self.strategy.category_weights.clone(),
                overview,
            ))
        } else {
            None
        };

        UserCommand::SaveAndExitSettings { llm, strategy }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub fn view<'a>(screen: &'a SettingsScreen) -> Element<'a, SettingsMessage> {
    // --- Tab bar ---
    let tabs = vec![
        Tab::new("LLM Config", SettingsMessage::SectionSelected(SettingsSection::LlmConfig)),
        Tab::new(
            "Strategy",
            SettingsMessage::SectionSelected(SettingsSection::StrategyConfig),
        ),
    ];

    let selected_idx = match screen.active_section {
        SettingsSection::LlmConfig => 0,
        SettingsSection::StrategyConfig => 1,
    };

    let tabs_el: Element<'a, SettingsMessage> =
        tab_bar(tabs, selected_idx, TabBarStyle::default()).into();

    // --- Section content ---
    let section_content: Element<'a, SettingsMessage> = match screen.active_section {
        SettingsSection::LlmConfig => llm_section::view(&screen.llm),
        SettingsSection::StrategyConfig => strategy_section::view(&screen.strategy),
    };

    // --- Action buttons ---
    let cancel_btn: Element<'a, SettingsMessage> = button(
        text("Cancel", TextStyle::default()),
        SettingsMessage::CancelRequested,
        ButtonStyle::new().variant(ButtonVariant::Ghost),
    )
    .into();

    let save_btn: Element<'a, SettingsMessage> = button(
        text("Save & Close", TextStyle::default()),
        SettingsMessage::SaveRequested,
        ButtonStyle::new().variant(ButtonVariant::Filled),
    )
    .into();

    let spacer: Element<'a, SettingsMessage> =
        iced::widget::Space::new().width(Length::Fill).into();

    let action_row: Element<'a, SettingsMessage> = h_stack(
        vec![cancel_btn, spacer, save_btn],
        StackStyle {
            gap: StackGap::Sm,
            align: StackAlign::Center,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    // --- Card body ---
    let title: Element<'a, SettingsMessage> = text(
        "Settings",
        TextStyle {
            size: TextSize::Xl2,
            weight: TextWeight::Bold,
            ..Default::default()
        },
    )
    .into();

    let subtitle: Element<'a, SettingsMessage> = text(
        "Configure LLM provider and draft strategy.",
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();

    let divider: Element<'a, SettingsMessage> = frame(
        iced::widget::Space::new().width(Length::Fill),
        BoxStyle {
            background: Some(Colors::BorderSubtle),
            width: Length::Fill,
            height: Length::Fixed(1.0),
            ..Default::default()
        },
    )
    .into();

    let card_body: Element<'a, SettingsMessage> = v_stack(
        vec![title, subtitle, tabs_el, section_content, divider, action_row],
        StackStyle {
            gap: StackGap::Md,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    let card: Element<'a, SettingsMessage> = section_box(card_body).into();

    // --- Discard modal overlay ---
    let base: Element<'a, SettingsMessage> = iced::widget::container(card)
        .max_width(640.0)
        .padding(Padding::new(24.0))
        .into();

    let centered: Element<'a, SettingsMessage> = iced::widget::container(base)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced::alignment::Horizontal::Center)
        .align_y(iced::alignment::Vertical::Center)
        .into();

    let modal = unsaved_modal::view(screen.discard_modal_open);
    crate::widgets::with_overlay(centered, modal)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use wyncast_core::llm::provider::LlmProvider;

    #[test]
    fn new_screen_is_clean() {
        let s = SettingsScreen::new();
        assert!(!s.is_dirty());
        assert!(!s.discard_modal_open);
        assert_eq!(s.active_section, SettingsSection::LlmConfig);
    }

    #[test]
    fn llm_form_change_marks_dirty() {
        let mut s = SettingsScreen::new();
        let _ = s.update(SettingsMessage::LlmFormChanged(LlmFormMessage::ApiKeyChanged(
            "new-key".to_string(),
        )));
        assert!(s.llm_dirty);
        assert!(s.is_dirty());
    }

    #[test]
    fn strategy_form_change_marks_dirty() {
        let mut s = SettingsScreen::new();
        let _ = s.update(SettingsMessage::StrategyFormChanged(
            StrategyFormMessage::HittingBudgetChanged(0.6),
        ));
        assert!(s.strategy_dirty);
        assert!(s.is_dirty());
    }

    #[test]
    fn cancel_when_clean_emits_exit() {
        let mut s = SettingsScreen::new();
        let (_, cmds) = s.update(SettingsMessage::CancelRequested);
        assert!(cmds.contains(&UserCommand::ExitSettings));
        assert!(!s.discard_modal_open);
    }

    #[test]
    fn cancel_when_dirty_opens_modal() {
        let mut s = SettingsScreen::new();
        s.llm_dirty = true;
        let (_, cmds) = s.update(SettingsMessage::CancelRequested);
        assert!(cmds.is_empty());
        assert!(s.discard_modal_open);
    }

    #[test]
    fn discard_confirmed_exits_and_resets() {
        let mut s = SettingsScreen::new();
        s.llm_dirty = true;
        s.discard_modal_open = true;
        let (_, cmds) = s.update(SettingsMessage::DiscardConfirmed);
        assert!(cmds.contains(&UserCommand::ExitSettings));
        assert!(!s.is_dirty());
        assert!(!s.discard_modal_open);
    }

    #[test]
    fn discard_cancelled_closes_modal_only() {
        let mut s = SettingsScreen::new();
        s.discard_modal_open = true;
        let _ = s.update(SettingsMessage::DiscardCancelled);
        assert!(!s.discard_modal_open);
    }

    #[test]
    fn section_selected_emits_switch_tab() {
        let mut s = SettingsScreen::new();
        let (_, cmds) = s.update(SettingsMessage::SectionSelected(SettingsSection::StrategyConfig));
        assert_eq!(s.active_section, SettingsSection::StrategyConfig);
        assert!(cmds
            .iter()
            .any(|c| matches!(c, UserCommand::SwitchSettingsTab(SettingsSection::StrategyConfig))));
    }

    #[test]
    fn save_with_dirty_llm_builds_command() {
        let mut s = SettingsScreen::new();
        s.llm.provider = Some(LlmProvider::Anthropic);
        s.llm.model_id = Some("claude-sonnet-4-6".to_string());
        s.llm.api_key = "sk-ant-test".to_string();
        s.llm_dirty = true;
        let cmd = s.build_save_command();
        assert!(matches!(cmd, UserCommand::SaveAndExitSettings { llm: Some(_), .. }));
    }

    #[test]
    fn save_with_clean_state_sends_empty_payloads() {
        let s = SettingsScreen::new();
        let cmd = s.build_save_command();
        assert!(matches!(cmd, UserCommand::SaveAndExitSettings { llm: None, strategy: None }));
    }

    #[test]
    fn apply_progress_sync_populates_and_clears_dirty() {
        let mut s = SettingsScreen::new();
        s.llm_dirty = true;
        s.apply_update(&OnboardingUpdate::ProgressSync {
            provider: Some(LlmProvider::Anthropic),
            model: Some("claude-sonnet-4-6".to_string()),
            api_key_mask: Some("sk-ant-•••6789".to_string()),
        });
        assert_eq!(s.llm.provider, Some(LlmProvider::Anthropic));
        assert_eq!(s.llm.model_id.as_deref(), Some("claude-sonnet-4-6"));
        assert!(!s.llm_dirty);
    }
}
