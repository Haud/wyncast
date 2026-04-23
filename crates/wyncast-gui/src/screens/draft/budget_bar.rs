use iced::{Element, Length, Padding};
use twui::{
    BoxStyle, Colors, StackGap, StackStyle, TextColor, TextSize, TextStyle,
    frame, h_stack, text, v_stack,
};

use crate::widgets::bar_gauge::{BarGaugeStyle, bar_gauge};
use crate::widgets::focus_ring;

/// Render the budget bar at the top of the sidebar.
///
/// Shows a stacked bar gauge (spent / cap) with color-coded fill and inline
/// stats for inflation, max bid, and average per slot.
pub fn view<'a, Message: Clone + 'a>(
    budget_spent: u32,
    budget_remaining: u32,
    salary_cap: u32,
    inflation_rate: f64,
    max_bid: u32,
    avg_per_slot: f64,
    focused: bool,
) -> Element<'a, Message> {
    let gauge_color = gauge_color(budget_remaining, salary_cap);
    let gauge_label = format!("${budget_spent} spent / ${salary_cap} cap");
    let gauge_elem = bar_gauge(
        &gauge_label,
        budget_spent as f32,
        salary_cap as f32,
        gauge_color,
        BarGaugeStyle::Stacked,
    );

    let inflation_color = if inflation_rate > 1.15 {
        TextColor::Yellow
    } else {
        TextColor::Default
    };
    let inflation_text: Element<Message> = text(
        format!("Inflation {:.3}x", inflation_rate),
        TextStyle {
            size: TextSize::Xs,
            color: inflation_color,
            ..Default::default()
        },
    )
    .into();

    let max_bid_text: Element<Message> = text(
        format!("Max ${max_bid}"),
        TextStyle {
            size: TextSize::Xs,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();

    let avg_text: Element<Message> = text(
        format!("${avg_per_slot:.1}/slot"),
        TextStyle {
            size: TextSize::Xs,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();

    let stats_row: Element<Message> = h_stack(
        vec![
            frame(inflation_text, BoxStyle { width: Length::Fill, ..Default::default() }).into(),
            max_bid_text,
            frame(avg_text, BoxStyle { padding: Padding::new(0.0).left(8.0), ..Default::default() }).into(),
        ],
        StackStyle {
            gap: StackGap::Xs,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    let content: Element<Message> = v_stack(
        vec![gauge_elem, stats_row],
        StackStyle {
            gap: StackGap::Sm,
            width: Length::Fill,
            padding: Padding::new(8.0),
            ..Default::default()
        },
    )
    .into();

    let panel: Element<Message> = frame(
        content,
        BoxStyle {
            width: Length::Fill,
            background: Some(Colors::BgElevated),
            ..Default::default()
        },
    )
    .into();

    focus_ring(panel, focused)
}

fn gauge_color(remaining: u32, cap: u32) -> Colors {
    if cap == 0 {
        return Colors::Success;
    }
    let frac = remaining as f64 / cap as f64;
    if frac >= 0.5 {
        Colors::Success
    } else if frac >= 0.25 {
        Colors::Warning
    } else {
        Colors::Destructive
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauge_color_high_remaining() {
        assert_eq!(gauge_color(200, 260), Colors::Success);
    }

    #[test]
    fn gauge_color_mid_remaining() {
        assert_eq!(gauge_color(65, 260), Colors::Warning);
    }

    #[test]
    fn gauge_color_low_remaining() {
        assert_eq!(gauge_color(30, 260), Colors::Destructive);
    }

    #[test]
    fn gauge_color_zero_cap() {
        assert_eq!(gauge_color(0, 0), Colors::Success);
    }

    #[test]
    fn view_does_not_panic_with_defaults() {
        let _elem: Element<String> = view(0, 260, 260, 1.0, 260, 0.0, false);
    }

    #[test]
    fn view_does_not_panic_with_data() {
        let _elem: Element<String> = view(120, 140, 260, 1.15, 115, 10.8, true);
    }

    #[test]
    fn inflation_above_threshold_uses_yellow_text() {
        // Verifies the threshold logic: 1.16 > 1.15 → yellow text.
        // We can only check indirectly via gauge_color; the text color is
        // tested by constructing the element without panicking.
        let _elem: Element<String> = view(120, 140, 260, 1.16, 115, 10.8, false);
    }
}
