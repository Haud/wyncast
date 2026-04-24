// LLM configuration step of the onboarding wizard.
//
// Delegates form fields to forms::llm_form and adds onboarding-specific navigation.

use iced::{Element, Length};
use twui::{
    BoxStyle, ButtonStyle, ButtonVariant, Colors, StackAlign, StackGap, StackStyle, TextColor,
    TextSize, TextStyle, TextWeight, button, frame, h_stack, section_box, text, v_stack,
};

use crate::forms::llm_form::LlmFormState;
use super::OnboardingMessage;

pub fn view_llm_setup<'a>(state: &'a LlmFormState) -> Element<'a, OnboardingMessage> {
    let title: Element<'a, OnboardingMessage> = text(
        "Configure LLM Provider",
        TextStyle {
            size: TextSize::Xl2,
            weight: TextWeight::Bold,
            ..Default::default()
        },
    )
    .into();

    let subtitle: Element<'a, OnboardingMessage> = text(
        "Choose your AI provider, select a model, and enter your API key.",
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();

    // Form fields — emits LlmFormMessage; map to OnboardingMessage::Llm
    let form_fields: Element<'a, OnboardingMessage> = crate::forms::llm_form::view(state)
        .map(OnboardingMessage::Llm);

    // Navigation buttons
    let skip_btn: Element<'a, OnboardingMessage> = button(
        text("Skip", TextStyle::default()),
        OnboardingMessage::Skip,
        ButtonStyle::new().variant(ButtonVariant::Ghost),
    )
    .into();

    let next_btn: Element<'a, OnboardingMessage> = button(
        text("Next →", TextStyle::default()),
        OnboardingMessage::Next,
        ButtonStyle::new()
            .variant(ButtonVariant::Filled)
            .disabled(!state.is_valid()),
    )
    .into();

    let spacer: Element<'a, OnboardingMessage> =
        iced::widget::Space::new().width(Length::Fill).into();

    let nav_row: Element<'a, OnboardingMessage> = h_stack(
        vec![skip_btn, spacer, next_btn],
        StackStyle {
            gap: StackGap::Sm,
            align: StackAlign::Center,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    let divider: Element<'a, OnboardingMessage> = frame(
        iced::widget::Space::new().width(Length::Fill),
        BoxStyle {
            background: Some(Colors::BorderSubtle),
            width: Length::Fill,
            height: Length::Fixed(1.0),
            ..Default::default()
        },
    )
    .into();

    let form_body: Element<'a, OnboardingMessage> = v_stack(
        vec![title, subtitle, form_fields, divider, nav_row],
        StackStyle {
            gap: StackGap::Md,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    section_box(form_body).into()
}
