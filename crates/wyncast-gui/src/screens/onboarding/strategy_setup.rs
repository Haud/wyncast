// Strategy configuration step of the onboarding wizard.

use iced::{Element, Length};
use twui::{
    BoxStyle, ButtonStyle, ButtonVariant, Colors, SliderStyle, StackAlign, StackGap, StackStyle,
    TextColor, TextSize, TextStyle, TextWeight, button, frame, h_stack, section_box, slider, text,
    text_field, v_stack, TextFieldStyle,
};
use wyncast_app::onboarding::strategy_config::{CategoryWeights, default_categories};

use super::OnboardingMessage;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct StrategyState {
    pub hitting_budget_pct: u8,
    pub category_weights: CategoryWeights,
    pub strategy_overview: String,
}

impl Default for StrategyState {
    fn default() -> Self {
        Self {
            hitting_budget_pct: 65,
            category_weights: CategoryWeights::default(),
            strategy_overview: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub fn view_strategy_setup<'a>(state: &'a StrategyState) -> Element<'a, OnboardingMessage> {
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

    // --- Hitting budget slider ---
    let budget_label: Element<'a, OnboardingMessage> = text(
        format!("HITTING BUDGET: {}%", state.hitting_budget_pct),
        TextStyle {
            size: TextSize::Xs,
            weight: TextWeight::Semibold,
            color: TextColor::Yellow,
            ..Default::default()
        },
    )
    .into();

    let budget_value = state.hitting_budget_pct as f32 / 100.0;
    let budget_slider: Element<'a, OnboardingMessage> =
        slider(budget_value, OnboardingMessage::HittingBudgetChanged, None, true, SliderStyle::new()).into();

    let budget_section: Element<'a, OnboardingMessage> = v_stack(
        vec![budget_label, budget_slider],
        StackStyle {
            gap: StackGap::Xs,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    // --- Category weights ---
    let weights_label: Element<'a, OnboardingMessage> = text(
        "CATEGORY WEIGHTS",
        TextStyle {
            size: TextSize::Xs,
            weight: TextWeight::Semibold,
            color: TextColor::Yellow,
            ..Default::default()
        },
    )
    .into();

    let categories = default_categories();
    let mut weight_items: Vec<Element<'a, OnboardingMessage>> = vec![weights_label];

    for (idx, cat) in categories.iter().enumerate() {
        let weight = state.category_weights.get(idx);
        let cat_label: Element<'a, OnboardingMessage> = iced::widget::container(
            text(
                format!("{} ({:.2}x)", cat, weight),
                TextStyle {
                    size: TextSize::Sm,
                    ..Default::default()
                },
            ),
        )
        .width(Length::Fixed(90.0))
        .into();

        // Normalize weight (0.0–2.0) to slider range (0.0–1.0)
        let normalized = weight / 2.0;
        let weight_slider: Element<'a, OnboardingMessage> = slider(
            normalized,
            move |v| OnboardingMessage::WeightChanged(idx, v * 2.0),
            None,
            false,
            SliderStyle::new(),
        )
        .into();

        let row: Element<'a, OnboardingMessage> = h_stack(
            vec![cat_label, weight_slider],
            StackStyle {
                gap: StackGap::Sm,
                align: StackAlign::Center,
                width: Length::Fill,
                ..Default::default()
            },
        )
        .into();

        weight_items.push(row);
    }

    let weights_section: Element<'a, OnboardingMessage> = v_stack(
        weight_items,
        StackStyle {
            gap: StackGap::Xs,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    // --- Strategy overview text field ---
    let overview_field: Element<'a, OnboardingMessage> = text_field(
        &state.strategy_overview,
        OnboardingMessage::StrategyOverviewChanged,
        TextFieldStyle::new()
            .label("STRATEGY OVERVIEW (OPTIONAL)")
            .placeholder("Describe your strategy goals…"),
    )
    .into();

    // --- Navigation ---
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

    let spacer: Element<'a, OnboardingMessage> = iced::widget::Space::new().width(Length::Fill).into();

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
        vec![
            title,
            subtitle,
            budget_section,
            weights_section,
            overview_field,
            divider_el,
            nav_row,
        ],
        StackStyle {
            gap: StackGap::Md,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    section_box(form_body).into()
}
