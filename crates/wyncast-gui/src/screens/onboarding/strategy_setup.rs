// Strategy configuration step of the onboarding wizard.
//
// Delegates form fields to forms::strategy_form and adds onboarding-specific navigation.

use iced::{Element, Length};
use twui::{
    BoxStyle, ButtonStyle, ButtonVariant, Colors, StackAlign, StackGap, StackStyle, TextColor,
    TextSize, TextStyle, TextWeight, button, frame, h_stack, section_box, text, v_stack,
};

use crate::forms::strategy_form::StrategyFormState;
use super::OnboardingMessage;

pub fn view_strategy_setup<'a>(state: &'a StrategyFormState) -> Element<'a, OnboardingMessage> {
    let title: Element<'a, OnboardingMessage> = text(
        "Draft Strategy",
        TextStyle {
            size: TextSize::Xl2,
            weight: TextWeight::Bold,
            ..Default::default()
        },
    )
    .into();

    let subtitle: Element<'a, OnboardingMessage> = text(
        "Set your hitting/pitching budget split and category weights.",
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();

    // Form fields — emits StrategyFormMessage; map to OnboardingMessage::Strategy
    let form_fields: Element<'a, OnboardingMessage> = crate::forms::strategy_form::view(state)
        .map(OnboardingMessage::Strategy);

    // Navigation buttons
    let back_btn: Element<'a, OnboardingMessage> = button(
        text("← Back", TextStyle::default()),
        OnboardingMessage::Back,
        ButtonStyle::new().variant(ButtonVariant::Ghost),
    )
    .into();

    let next_btn: Element<'a, OnboardingMessage> = button(
        text("Next →", TextStyle::default()),
        OnboardingMessage::Next,
        ButtonStyle::new().variant(ButtonVariant::Filled),
    )
    .into();

    let spacer: Element<'a, OnboardingMessage> =
        iced::widget::Space::new().width(Length::Fill).into();

    let nav_row: Element<'a, OnboardingMessage> = h_stack(
        vec![back_btn, spacer, next_btn],
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
