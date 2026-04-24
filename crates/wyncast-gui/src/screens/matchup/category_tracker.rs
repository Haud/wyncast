use iced::widget::operation::{self, AbsoluteOffset};
use iced::widget::Id as WidgetId;
use iced::{Background, Element, Length, Padding, Task};
use twui::{
    BoxStyle, Colors, StackGap, StackStyle, TextColor, TextSize, TextStyle,
    frame, h_stack, text, v_stack,
};
use wyncast_app::protocol::ScrollDirection;
use wyncast_baseball::matchup::{CategoryScore, CategoryState};

use crate::widgets::data_table::ROW_HEIGHT;
use crate::widgets::focus_ring::focus_ring;

#[derive(Debug, Clone)]
pub enum CategoryTrackerMessage {
    ScrollBy(ScrollDirection),
}

pub struct CategoryTracker {
    scroll_id: WidgetId,
}

impl CategoryTracker {
    pub fn new() -> Self {
        Self { scroll_id: WidgetId::unique() }
    }

    pub fn update(&mut self, msg: CategoryTrackerMessage) -> Task<CategoryTrackerMessage> {
        match msg {
            CategoryTrackerMessage::ScrollBy(dir) => {
                let dy = scroll_delta(dir);
                operation::scroll_by(self.scroll_id.clone(), AbsoluteOffset { x: 0.0, y: dy })
            }
        }
    }

    pub fn view<'a>(
        &self,
        scores: &'a [CategoryScore],
        focused: bool,
    ) -> Element<'a, CategoryTrackerMessage> {
        let title: Element<CategoryTrackerMessage> = text(
            "Category Tracker",
            TextStyle {
                size: TextSize::Sm,
                color: TextColor::Dimmed,
                ..Default::default()
            },
        )
        .into();

        let rows: Vec<Element<CategoryTrackerMessage>> = scores
            .iter()
            .map(category_row)
            .collect();

        let mut children: Vec<Element<CategoryTrackerMessage>> = vec![title];
        children.extend(rows);

        let body: Element<CategoryTrackerMessage> = v_stack(
            children,
            StackStyle {
                gap: StackGap::Xs,
                width: Length::Fill,
                height: Length::Fill,
                padding: Padding::new(4.0),
                background: Some(Colors::BgSidebar),
                ..Default::default()
            },
        )
        .into();

        let scrollable: Element<CategoryTrackerMessage> =
            iced::widget::scrollable::Scrollable::new(body)
                .id(self.scroll_id.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .into();

        focus_ring(scrollable, focused)
    }
}

impl Default for CategoryTracker {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Category row rendering
// ---------------------------------------------------------------------------

fn category_row<'a>(score: &'a CategoryScore) -> Element<'a, CategoryTrackerMessage> {
    let home_share = compute_home_share(score.home_value, score.away_value, &score.state);

    let (home_color, away_color) = match score.state {
        CategoryState::HomeWinning => (Colors::Success, Colors::Destructive),
        CategoryState::AwayWinning => (Colors::Destructive, Colors::Success),
        CategoryState::Tied => (Colors::Warning, Colors::Warning),
    };

    // Label (stat abbreviation)
    let label: Element<CategoryTrackerMessage> = frame(
        text(
            score.stat_abbrev.clone(),
            TextStyle {
                size: TextSize::Xs,
                color: TextColor::Default,
                ..Default::default()
            },
        ),
        BoxStyle {
            width: Length::Fixed(40.0),
            padding: Padding::new(2.0),
            ..Default::default()
        },
    )
    .into();

    // Two-segment proportional bar
    let scale: u16 = 1000;
    let home_w = ((home_share as f32 * scale as f32).round() as u16).clamp(1, scale - 1);
    let away_w = scale - home_w;

    let home_seg: Element<CategoryTrackerMessage> = iced::widget::container(
        iced::widget::Space::new(),
    )
    .width(Length::FillPortion(home_w))
    .height(Length::Fixed(8.0))
    .style(move |_| iced::widget::container::Style {
        background: Some(Background::Color(home_color.rgb())),
        ..Default::default()
    })
    .into();

    let away_seg: Element<CategoryTrackerMessage> = iced::widget::container(
        iced::widget::Space::new(),
    )
    .width(Length::FillPortion(away_w))
    .height(Length::Fixed(8.0))
    .style(move |_| iced::widget::container::Style {
        background: Some(Background::Color(away_color.rgb())),
        ..Default::default()
    })
    .into();

    let bar: Element<CategoryTrackerMessage> = iced::widget::Row::new()
        .push(home_seg)
        .push(away_seg)
        .height(Length::Fixed(8.0))
        .width(Length::Fill)
        .into();

    // Status chip (who's winning)
    let status_str = match score.state {
        CategoryState::HomeWinning => "HOME",
        CategoryState::AwayWinning => "AWAY",
        CategoryState::Tied => "TIE",
    };
    let status_color = match score.state {
        CategoryState::HomeWinning => TextColor::Default,
        CategoryState::AwayWinning => TextColor::Error,
        CategoryState::Tied => TextColor::Yellow,
    };
    let status: Element<CategoryTrackerMessage> = frame(
        text(
            status_str,
            TextStyle {
                size: TextSize::Xs,
                color: status_color,
                ..Default::default()
            },
        ),
        BoxStyle {
            width: Length::Fixed(40.0),
            padding: Padding::new(2.0),
            ..Default::default()
        },
    )
    .into();

    let bar_with_ends: Element<CategoryTrackerMessage> = h_stack(
        vec![label, bar, status],
        StackStyle {
            gap: StackGap::Xs,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    // Values row
    let home_val_str = format_value(score.home_value, &score.stat_abbrev);
    let away_val_str = format_value(score.away_value, &score.stat_abbrev);

    let home_val: Element<CategoryTrackerMessage> = text(
        home_val_str,
        TextStyle {
            size: TextSize::Xs,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();
    let away_val: Element<CategoryTrackerMessage> = text(
        away_val_str,
        TextStyle {
            size: TextSize::Xs,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();

    let spacer: Element<CategoryTrackerMessage> = iced::widget::Space::new()
        .width(Length::Fill)
        .into();

    let vals: Element<CategoryTrackerMessage> = h_stack(
        vec![home_val, spacer, away_val],
        StackStyle {
            gap: StackGap::None,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    v_stack(
        vec![bar_with_ends, vals],
        StackStyle {
            gap: StackGap::None,
            width: Length::Fill,
            padding: Padding::new(2.0),
            ..Default::default()
        },
    )
    .into()
}

/// Compute the home-side proportion for the visual bar.
///
/// For higher-is-better: home_share = home / (home + away)
/// For lower-is-better (inferred from state when home has lower value but is winning):
///   home_share = away / (home + away)
/// Uses CategoryState to determine direction without needing stat registry.
pub fn compute_home_share(home_value: f64, away_value: f64, state: &CategoryState) -> f64 {
    let total = home_value.abs() + away_value.abs();
    if total == 0.0 {
        return 0.5;
    }

    let raw = home_value.abs() / total;

    match state {
        CategoryState::Tied => 0.5,
        CategoryState::HomeWinning => {
            // Home is winning; home_share should be >= 0.5
            if raw >= 0.5 { raw } else { 1.0 - raw }
        }
        CategoryState::AwayWinning => {
            // Away is winning; home_share should be <= 0.5
            if raw <= 0.5 { raw } else { 1.0 - raw }
        }
    }
}

fn format_value(val: f64, abbrev: &str) -> String {
    match abbrev {
        "AVG" | "OBP" | "SLG" | "OPS" => format!("{val:.3}"),
        "ERA" | "WHIP" | "K/9" | "BB/9" | "K/BB" => format!("{val:.2}"),
        "IP" => format!("{val:.1}"),
        _ => format!("{}", val as i64),
    }
}

fn scroll_delta(dir: ScrollDirection) -> f32 {
    match dir {
        ScrollDirection::Up => -(ROW_HEIGHT * 3.0),
        ScrollDirection::Down => ROW_HEIGHT * 3.0,
        ScrollDirection::PageUp => -(ROW_HEIGHT * 10.0),
        ScrollDirection::PageDown => ROW_HEIGHT * 10.0,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_share_home_winning_higher_is_better() {
        let share = compute_home_share(8.0, 2.0, &CategoryState::HomeWinning);
        assert!(share >= 0.5, "home winning should give share >= 0.5, got {share}");
        assert!((share - 0.8).abs() < 1e-9);
    }

    #[test]
    fn home_share_away_winning_higher_is_better() {
        let share = compute_home_share(2.0, 8.0, &CategoryState::AwayWinning);
        assert!(share <= 0.5, "away winning should give home share <= 0.5, got {share}");
        assert!((share - 0.2).abs() < 1e-9);
    }

    #[test]
    fn home_share_home_winning_lower_is_better() {
        // ERA: home 2.5, away 4.5 — home wins (lower ERA)
        // raw = 2.5/7.0 ≈ 0.357 < 0.5, so we flip: 1 - 0.357 = 0.643
        let share = compute_home_share(2.5, 4.5, &CategoryState::HomeWinning);
        assert!(share >= 0.5, "home winning lower-is-better should flip share to >= 0.5, got {share}");
    }

    #[test]
    fn home_share_tied() {
        let share = compute_home_share(5.0, 5.0, &CategoryState::Tied);
        assert!((share - 0.5).abs() < 1e-9);
    }

    #[test]
    fn home_share_both_zero() {
        let share = compute_home_share(0.0, 0.0, &CategoryState::Tied);
        assert!((share - 0.5).abs() < 1e-9);
    }

    #[test]
    fn format_value_avg() {
        assert_eq!(format_value(0.283, "AVG"), "0.283");
    }

    #[test]
    fn format_value_counting() {
        assert_eq!(format_value(5.0, "R"), "5");
    }

    #[test]
    fn format_value_era() {
        assert_eq!(format_value(3.45, "ERA"), "3.45");
    }
}
