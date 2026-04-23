use iced::widget::operation::{self, AbsoluteOffset};
use iced::widget::scrollable;
use iced::widget::Id as ScrollId;
use iced::{Element, Length, Padding, Task};
use wyncast_app::protocol::ScrollDirection;
use wyncast_baseball::valuation::scarcity::{ScarcityEntry, ScarcityUrgency};
use twui::{
    BoxStyle, Colors, Opacity, StackGap, StackStyle, TextColor, TextSize, TextStyle, frame, text,
    v_stack,
};

use crate::widgets::focus_ring;
use crate::widgets::bar_gauge::{BarGaugeStyle, bar_gauge};

// Upper bound for the gauge: 20+ players above replacement = full bar.
const SCARCITY_MAX: f32 = 20.0;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct ScarcityPanel {
    scroll_id: ScrollId,
}

impl ScarcityPanel {
    pub fn new() -> Self {
        Self { scroll_id: ScrollId::unique() }
    }

    pub fn scroll_by<M: 'static>(&self, dir: ScrollDirection) -> Task<M> {
        let (dx, dy) = scroll_amount(dir);
        operation::scroll_by(self.scroll_id.clone(), AbsoluteOffset { x: dx, y: dy })
    }

    pub fn view<'a, Message: Clone + 'a>(
        &'a self,
        focused: bool,
        entries: &'a [ScarcityEntry],
        nominated_position: Option<&str>,
    ) -> Element<'a, Message> {
        let rows: Vec<Element<Message>> = entries
            .iter()
            .map(|entry| {
                let is_nominated = nominated_position
                    .map(|np| entry.position.display_str() == np)
                    .unwrap_or(false);
                scarcity_row(entry, is_nominated)
            })
            .collect();

        let list: Element<Message> = if rows.is_empty() {
            let placeholder: Element<Message> = text(
                "No scarcity data",
                TextStyle {
                    size: TextSize::Xs,
                    color: TextColor::Dimmed,
                    ..Default::default()
                },
            )
            .into();
            frame(placeholder, BoxStyle { padding: Padding::new(8.0), ..Default::default() })
                .into()
        } else {
            v_stack(
                rows,
                StackStyle {
                    gap: StackGap::Xs,
                    width: Length::Fill,
                    padding: Padding::new(4.0),
                    ..Default::default()
                },
            )
            .into()
        };

        let scrollable = scrollable::Scrollable::new(list)
            .id(self.scroll_id.clone())
            .width(Length::Fill)
            .height(Length::Fill);

        let scrollable_elem: Element<'a, Message> = scrollable.into();
        let panel = frame(
            scrollable_elem,
            BoxStyle {
                width: Length::Fill,
                height: Length::Fill,
                padding: Padding::new(4.0),
                background: Some(Colors::BgElevated),
                ..Default::default()
            },
        );

        let panel_elem: Element<'a, Message> = panel.into();
        focus_ring(panel_elem, focused)
    }
}

// ---------------------------------------------------------------------------
// Row helpers
// ---------------------------------------------------------------------------

fn urgency_color(urgency: ScarcityUrgency) -> Colors {
    match urgency {
        ScarcityUrgency::Critical => Colors::Destructive,
        ScarcityUrgency::High => Colors::Warning,
        ScarcityUrgency::Medium => Colors::Primary,
        ScarcityUrgency::Low => Colors::Success,
    }
}

fn scarcity_row<'a, Message: Clone + 'a>(
    entry: &ScarcityEntry,
    is_nominated: bool,
) -> Element<'a, Message> {
    let color = urgency_color(entry.urgency);
    let gauge = bar_gauge(
        entry.position.display_str(),
        entry.players_above_replacement as f32,
        SCARCITY_MAX,
        color,
        BarGaugeStyle::Compact,
    );

    if is_nominated {
        let bg = Colors::Tertiary.rgba(Opacity::O20);
        iced::widget::container(gauge)
            .width(Length::Fill)
            .style(move |_| iced::widget::container::Style {
                background: Some(iced::Background::Color(bg)),
                ..Default::default()
            })
            .into()
    } else {
        gauge
    }
}

fn scroll_amount(dir: ScrollDirection) -> (f32, f32) {
    match dir {
        ScrollDirection::Up => (0.0, -24.0),
        ScrollDirection::Down => (0.0, 24.0),
        ScrollDirection::PageUp => (0.0, -200.0),
        ScrollDirection::PageDown => (0.0, 200.0),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urgency_color_critical() {
        assert_eq!(urgency_color(ScarcityUrgency::Critical), Colors::Destructive);
    }

    #[test]
    fn urgency_color_high() {
        assert_eq!(urgency_color(ScarcityUrgency::High), Colors::Warning);
    }

    #[test]
    fn urgency_color_medium() {
        assert_eq!(urgency_color(ScarcityUrgency::Medium), Colors::Primary);
    }

    #[test]
    fn urgency_color_low() {
        assert_eq!(urgency_color(ScarcityUrgency::Low), Colors::Success);
    }

    #[test]
    fn scarcity_panel_constructs() {
        let _ = ScarcityPanel::new();
    }

    #[test]
    fn scroll_up_is_negative() {
        let (_, dy) = scroll_amount(ScrollDirection::Up);
        assert!(dy < 0.0);
    }

    #[test]
    fn scroll_down_is_positive() {
        let (_, dy) = scroll_amount(ScrollDirection::Down);
        assert!(dy > 0.0);
    }
}
