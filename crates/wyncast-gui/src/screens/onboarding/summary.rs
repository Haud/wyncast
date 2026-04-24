// Summary / confirmation step shown briefly when onboarding is complete.
// This view is rendered while AppMode::Onboarding(Complete) is active, until
// the backend sends ModeChanged(Draft).

use iced::{Element, Length, Padding};
use twui::{
    BoxStyle, ButtonStyle, ButtonVariant, Colors, StackAlign, StackGap, StackStyle, TextColor,
    TextSize, TextStyle, TextWeight, button, frame, h_stack, section_box, text, v_stack,
};
use wyncast_core::llm::provider::LlmProvider;

use super::OnboardingMessage;

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub fn view_summary<'a>(
    provider: Option<&'a LlmProvider>,
    model_id: Option<&'a str>,
    hitting_budget_pct: u8,
) -> Element<'a, OnboardingMessage> {
    let title: Element<'a, OnboardingMessage> = text(
        "You're Ready to Draft!",
        TextStyle {
            size: TextSize::Xl2,
            weight: TextWeight::Bold,
            ..Default::default()
        },
    )
    .into();

    let subtitle: Element<'a, OnboardingMessage> = text(
        "Your configuration is saved. Launch the draft assistant to get started.",
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();

    let provider_name = provider.map(|p| p.display_name()).unwrap_or("None");
    let model_name = model_id.unwrap_or("None");

    let summary_content: Element<'a, OnboardingMessage> = v_stack(
        vec![
            summary_row("LLM Provider", provider_name),
            summary_row("Model", model_name),
            summary_row("Hitting Budget", format!("{}%", hitting_budget_pct)),
        ],
        StackStyle {
            gap: StackGap::Sm,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    let summary_box: Element<'a, OnboardingMessage> = frame(
        summary_content,
        BoxStyle {
            background: Some(Colors::BgElevated),
            padding: Padding::new(16.0),
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    let back_btn: Element<'a, OnboardingMessage> = button(
        text("← Back", TextStyle::default()),
        OnboardingMessage::Back,
        ButtonStyle::new().variant(ButtonVariant::Ghost),
    )
    .into();

    let finish_btn: Element<'a, OnboardingMessage> = button(
        text("Start Drafting →", TextStyle::default()),
        OnboardingMessage::Finish,
        ButtonStyle::new().variant(ButtonVariant::Filled),
    )
    .into();

    let spacer: Element<'a, OnboardingMessage> = iced::widget::Space::new().width(Length::Fill).into();

    let nav_row: Element<'a, OnboardingMessage> = h_stack(
        vec![back_btn, spacer, finish_btn],
        StackStyle {
            gap: StackGap::Sm,
            align: StackAlign::Center,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    let divider_el: Element<'a, OnboardingMessage> = frame(
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
        vec![title, subtitle, summary_box, divider_el, nav_row],
        StackStyle {
            gap: StackGap::Md,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    section_box(form_body).into()
}

fn summary_row<'a>(label: &'a str, value: impl ToString) -> Element<'a, OnboardingMessage> {
    let label_el: Element<'a, OnboardingMessage> = text(
        label,
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();

    let spacer: Element<'a, OnboardingMessage> = iced::widget::Space::new().width(Length::Fill).into();

    let value_el: Element<'a, OnboardingMessage> = text(
        value,
        TextStyle {
            size: TextSize::Sm,
            weight: TextWeight::Semibold,
            ..Default::default()
        },
    )
    .into();

    h_stack(
        vec![label_el, spacer, value_el],
        StackStyle {
            gap: StackGap::Sm,
            align: StackAlign::Center,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into()
}
