// Shared strategy configuration form — used by both onboarding wizard and settings screen.

use iced::{Element, Length};
use twui::{
    BoxStyle, Colors, SliderStyle, StackAlign, StackGap, StackStyle, TextColor, TextSize,
    TextStyle, TextWeight, frame, h_stack, slider, text, text_field, v_stack, TextFieldStyle,
};
use wyncast_app::onboarding::strategy_config::{CategoryWeights, default_categories};

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum StrategyFormMessage {
    HittingBudgetChanged(f32),
    WeightChanged(usize, f32),
    StrategyOverviewChanged(String),
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StrategyFormState {
    pub hitting_budget_pct: u8,
    pub category_weights: CategoryWeights,
    pub strategy_overview: String,
}

impl Default for StrategyFormState {
    fn default() -> Self {
        Self {
            hitting_budget_pct: 65,
            category_weights: CategoryWeights::default(),
            strategy_overview: String::new(),
        }
    }
}

impl StrategyFormState {
    /// Apply a form message; returns true if a data field changed (dirty).
    pub fn apply(&mut self, msg: StrategyFormMessage) -> bool {
        match msg {
            StrategyFormMessage::HittingBudgetChanged(v) => {
                self.hitting_budget_pct = (v * 100.0).round() as u8;
                true
            }
            StrategyFormMessage::WeightChanged(idx, v) => {
                self.category_weights.set(idx, v);
                true
            }
            StrategyFormMessage::StrategyOverviewChanged(s) => {
                self.strategy_overview = s;
                true
            }
        }
    }
}

// ---------------------------------------------------------------------------
// View (form fields only — no navigation buttons)
// ---------------------------------------------------------------------------

pub fn view<'a>(state: &'a StrategyFormState) -> Element<'a, StrategyFormMessage> {
    // --- Hitting budget slider ---
    let budget_label: Element<'a, StrategyFormMessage> = text(
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
    let budget_slider: Element<'a, StrategyFormMessage> = slider(
        budget_value,
        StrategyFormMessage::HittingBudgetChanged,
        None,
        true,
        SliderStyle::new(),
    )
    .into();

    let budget_section: Element<'a, StrategyFormMessage> = v_stack(
        vec![budget_label, budget_slider],
        StackStyle {
            gap: StackGap::Xs,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    // --- Category weights ---
    let weights_label: Element<'a, StrategyFormMessage> = text(
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
    let mut weight_items: Vec<Element<'a, StrategyFormMessage>> = vec![weights_label];

    for (idx, cat) in categories.iter().enumerate() {
        let weight = state.category_weights.get(idx);
        let cat_label: Element<'a, StrategyFormMessage> = iced::widget::container(text(
            format!("{} ({:.2}x)", cat, weight),
            TextStyle {
                size: TextSize::Sm,
                ..Default::default()
            },
        ))
        .width(Length::Fixed(90.0))
        .into();

        let normalized = weight / 2.0;
        let weight_slider: Element<'a, StrategyFormMessage> = slider(
            normalized,
            move |v| StrategyFormMessage::WeightChanged(idx, v * 2.0),
            None,
            false,
            SliderStyle::new(),
        )
        .into();

        let row: Element<'a, StrategyFormMessage> = h_stack(
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

    let weights_section: Element<'a, StrategyFormMessage> = v_stack(
        weight_items,
        StackStyle {
            gap: StackGap::Xs,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    // --- Strategy overview text field ---
    let overview_field: Element<'a, StrategyFormMessage> = text_field(
        &state.strategy_overview,
        StrategyFormMessage::StrategyOverviewChanged,
        TextFieldStyle::new()
            .label("STRATEGY OVERVIEW (OPTIONAL)")
            .placeholder("Describe your strategy goals…"),
    )
    .into();

    // --- Divider ---
    let divider: Element<'a, StrategyFormMessage> = frame(
        iced::widget::Space::new().width(Length::Fill),
        BoxStyle {
            background: Some(Colors::BorderSubtle),
            width: Length::Fill,
            height: Length::Fixed(1.0),
            ..Default::default()
        },
    )
    .into();

    v_stack(
        vec![budget_section, weights_section, overview_field, divider],
        StackStyle {
            gap: StackGap::Md,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into()
}
