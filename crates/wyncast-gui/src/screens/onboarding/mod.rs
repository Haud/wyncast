// Onboarding wizard screen — Phase 3.10.
//
// Renders AppMode::Onboarding(step) as a full-screen centered wizard with a
// three-step flow: LLM config → Strategy config → Summary (Complete).
//
// Form state and messages are now shared with the settings screen via the
// forms/ module (Phase 3.11 refactor).

mod llm_setup;
mod strategy_setup;
mod summary;

use iced::{Element, Length, Padding, Task};
use twui::{
    BoxStyle, Colors, StackAlign, StackGap, StackStyle, TextColor, TextSize, TextStyle,
    TextWeight, frame, h_stack, text, v_stack,
};
use wyncast_app::onboarding::OnboardingStep;
use wyncast_app::protocol::{OnboardingAction, OnboardingUpdate, UserCommand};

use crate::forms::llm_form::{ConnectionTestState, LlmFormMessage, LlmFormState};
use crate::forms::strategy_form::{StrategyFormMessage, StrategyFormState};

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum OnboardingMessage {
    /// Messages from the LLM configuration form.
    Llm(LlmFormMessage),
    /// Messages from the strategy configuration form.
    Strategy(StrategyFormMessage),
    /// Navigation
    Next,
    Back,
    Finish,
    Skip,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct OnboardingScreen {
    pub llm: LlmFormState,
    pub strategy: StrategyFormState,
}

impl OnboardingScreen {
    pub fn new() -> Self {
        Self {
            llm: LlmFormState::default(),
            strategy: StrategyFormState::default(),
        }
    }

    pub fn apply_update(&mut self, update: OnboardingUpdate) {
        match update {
            OnboardingUpdate::ProgressSync { provider, model, api_key_mask } => {
                if let Some(p) = provider {
                    self.llm.provider = Some(p);
                }
                if let Some(m) = model {
                    self.llm.model_id = Some(m);
                }
                self.llm.api_key_mask = api_key_mask;
            }
            OnboardingUpdate::ConnectionTestResult { success, message } => {
                self.llm.connection_test = if success {
                    ConnectionTestState::Success
                } else {
                    ConnectionTestState::Failed
                };
                self.llm.connection_test_message = message;
            }
            _ => {}
        }
    }

    pub fn update(&mut self, msg: OnboardingMessage) -> (Task<OnboardingMessage>, Vec<UserCommand>) {
        let mut cmds = Vec::new();
        match msg {
            OnboardingMessage::Llm(llm_msg) => {
                let was_test = matches!(llm_msg, LlmFormMessage::TestConnection);
                self.llm.apply(llm_msg);
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
            OnboardingMessage::Strategy(strategy_msg) => {
                self.strategy.apply(strategy_msg);
            }
            OnboardingMessage::Next => {
                cmds.extend(self.handle_next());
            }
            OnboardingMessage::Back => {
                cmds.push(UserCommand::OnboardingAction(OnboardingAction::GoBack));
            }
            OnboardingMessage::Finish => {
                cmds.push(UserCommand::OnboardingAction(OnboardingAction::GoNext));
            }
            OnboardingMessage::Skip => {
                cmds.push(UserCommand::OnboardingAction(OnboardingAction::Skip));
            }
        }
        (Task::none(), cmds)
    }

    fn handle_next(&self) -> Vec<UserCommand> {
        let mut cmds = Vec::new();
        if let (Some(provider), Some(model_id)) = (&self.llm.provider, &self.llm.model_id) {
            let api_key = if self.llm.api_key.is_empty() {
                None
            } else {
                Some(self.llm.api_key.clone())
            };
            cmds.push(UserCommand::OnboardingAction(OnboardingAction::SaveLlmConfig {
                provider: provider.clone(),
                model_id: model_id.clone(),
                api_key,
            }));
        }
        cmds.push(UserCommand::OnboardingAction(OnboardingAction::GoNext));
        cmds
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub fn view<'a>(
    screen: &'a OnboardingScreen,
    step: &'a OnboardingStep,
) -> Element<'a, OnboardingMessage> {
    let step_indicator = build_step_indicator(step);

    let form: Element<'a, OnboardingMessage> = match step {
        OnboardingStep::LlmSetup => llm_setup::view_llm_setup(&screen.llm),
        OnboardingStep::StrategySetup => strategy_setup::view_strategy_setup(&screen.strategy),
        OnboardingStep::Complete => summary::view_summary(
            screen.llm.provider.as_ref(),
            screen.llm.model_id.as_deref(),
            screen.strategy.hitting_budget_pct,
        ),
    };

    let wizard: Element<'a, OnboardingMessage> = v_stack(
        vec![step_indicator, form],
        StackStyle {
            gap: StackGap::Lg,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    let constrained: Element<'a, OnboardingMessage> = iced::widget::container(wizard)
        .max_width(640.0)
        .padding(Padding::new(24.0))
        .into();

    iced::widget::container(constrained)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(iced::alignment::Horizontal::Center)
        .align_y(iced::alignment::Vertical::Center)
        .into()
}

fn build_step_indicator<'a>(step: &OnboardingStep) -> Element<'a, OnboardingMessage> {
    let steps = [("LLM Setup", 0usize), ("Strategy", 1usize), ("Ready", 2usize)];

    let current_idx: usize = match step {
        OnboardingStep::LlmSetup => 0,
        OnboardingStep::StrategySetup => 1,
        OnboardingStep::Complete => 2,
    };

    let mut items: Vec<Element<'a, OnboardingMessage>> = Vec::new();

    for (i, (label, _)) in steps.iter().enumerate() {
        let is_current = i == current_idx;
        let is_done = i < current_idx;

        let (dot_char, dot_color, text_color) = if is_done {
            ("✓", TextColor::Blue, TextColor::Dimmed)
        } else if is_current {
            ("●", TextColor::Default, TextColor::Default)
        } else {
            ("○", TextColor::Dimmed, TextColor::Dimmed)
        };

        let dot: Element<'a, OnboardingMessage> = text(
            dot_char,
            TextStyle {
                size: TextSize::Md,
                color: dot_color,
                ..Default::default()
            },
        )
        .into();

        let label_el: Element<'a, OnboardingMessage> = text(
            *label,
            TextStyle {
                size: TextSize::Xs,
                color: text_color,
                weight: if is_current {
                    TextWeight::Semibold
                } else {
                    TextWeight::Normal
                },
                ..Default::default()
            },
        )
        .into();

        let step_item: Element<'a, OnboardingMessage> = h_stack(
            vec![dot, label_el],
            StackStyle {
                gap: StackGap::Xs,
                align: StackAlign::Center,
                ..Default::default()
            },
        )
        .into();

        items.push(step_item);

        if i < steps.len() - 1 {
            let connector_color = if i < current_idx {
                Colors::Secondary
            } else {
                Colors::BgInput
            };
            let connector: Element<'a, OnboardingMessage> = frame(
                iced::widget::Space::new().width(Length::Fixed(32.0)),
                BoxStyle {
                    background: Some(connector_color),
                    height: Length::Fixed(1.0),
                    width: Length::Fixed(32.0),
                    ..Default::default()
                },
            )
            .into();
            items.push(connector);
        }
    }

    h_stack(
        items,
        StackStyle {
            gap: StackGap::Xs,
            align: StackAlign::Center,
            ..Default::default()
        },
    )
    .into()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use wyncast_core::llm::provider::LlmProvider;

    #[test]
    fn llm_form_invalid_when_empty() {
        let state = LlmFormState::default();
        assert!(!state.is_valid());
    }

    #[test]
    fn llm_form_valid_with_provider_model_key() {
        let mut state = LlmFormState::default();
        state.provider = Some(LlmProvider::Anthropic);
        state.model_id = Some("claude-sonnet-4-6".to_string());
        state.api_key = "sk-ant-test".to_string();
        assert!(state.is_valid());
    }

    #[test]
    fn llm_form_valid_with_saved_key_mask() {
        let mut state = LlmFormState::default();
        state.provider = Some(LlmProvider::Anthropic);
        state.model_id = Some("claude-sonnet-4-6".to_string());
        state.api_key_mask = Some("sk-ant-•••••6789".to_string());
        assert!(state.is_valid());
    }

    #[test]
    fn llm_form_invalid_missing_model() {
        let mut state = LlmFormState::default();
        state.provider = Some(LlmProvider::Anthropic);
        state.api_key = "sk-ant-test".to_string();
        assert!(!state.is_valid());
    }

    #[test]
    fn update_provider_resets_model() {
        let mut screen = OnboardingScreen::new();
        screen.llm.model_id = Some("claude-sonnet-4-6".to_string());
        let _ = screen.update(OnboardingMessage::Llm(LlmFormMessage::ProviderSelected(
            LlmProvider::Google,
        )));
        assert!(screen.llm.model_id.is_none());
        assert_eq!(screen.llm.provider, Some(LlmProvider::Google));
    }

    #[test]
    fn update_api_key_clears_test_result() {
        let mut screen = OnboardingScreen::new();
        screen.llm.connection_test = ConnectionTestState::Success;
        let _ = screen
            .update(OnboardingMessage::Llm(LlmFormMessage::ApiKeyChanged("new-key".to_string())));
        assert_eq!(screen.llm.connection_test, ConnectionTestState::Idle);
    }

    #[test]
    fn apply_progress_sync_fills_fields() {
        let mut screen = OnboardingScreen::new();
        screen.apply_update(OnboardingUpdate::ProgressSync {
            provider: Some(LlmProvider::Anthropic),
            model: Some("claude-sonnet-4-6".to_string()),
            api_key_mask: Some("sk-ant-•••••6789".to_string()),
        });
        assert_eq!(screen.llm.provider, Some(LlmProvider::Anthropic));
        assert_eq!(screen.llm.model_id.as_deref(), Some("claude-sonnet-4-6"));
        assert!(screen.llm.api_key_mask.is_some());
    }

    #[test]
    fn hitting_budget_changed_rounds() {
        let mut screen = OnboardingScreen::new();
        let _ = screen.update(OnboardingMessage::Strategy(
            StrategyFormMessage::HittingBudgetChanged(0.75),
        ));
        assert_eq!(screen.strategy.hitting_budget_pct, 75);
    }

    #[test]
    fn weight_changed_updates_category() {
        let mut screen = OnboardingScreen::new();
        let _ = screen.update(OnboardingMessage::Strategy(StrategyFormMessage::WeightChanged(
            0, 1.5,
        )));
        assert!((screen.strategy.category_weights.get(0) - 1.5).abs() < 0.001);
    }
}
