use iced::widget::operation::{self, AbsoluteOffset};
use iced::widget::scrollable;
use iced::widget::Id as ScrollId;
use iced::{Element, Length, Padding, Task};
use wyncast_app::protocol::ScrollDirection;
use wyncast_baseball::draft::pick::Position;
use wyncast_baseball::draft::roster::RosterSlot;
use twui::{
    BoxStyle, Colors, Opacity, StackGap, StackStyle, TextColor, TextSize, TextStyle, frame,
    h_stack, text, v_stack,
};

use crate::widgets::focus_ring;

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum RosterMessage {
    UserScrolled,
    ScrollBy(ScrollDirection),
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct RosterPanel {
    scroll_id: ScrollId,
}

impl RosterPanel {
    pub fn new() -> Self {
        Self { scroll_id: ScrollId::unique() }
    }

    pub fn update(&mut self, msg: RosterMessage) -> Task<RosterMessage> {
        match msg {
            RosterMessage::UserScrolled => Task::none(),
            RosterMessage::ScrollBy(dir) => {
                let (dx, dy) = scroll_amount(dir);
                operation::scroll_by(self.scroll_id.clone(), AbsoluteOffset { x: dx, y: dy })
            }
        }
    }

    pub fn view<'a>(
        &'a self,
        focused: bool,
        slots: &'a [RosterSlot],
        nominated_position: Option<&str>,
    ) -> Element<'a, RosterMessage> {
        let rows: Vec<Element<RosterMessage>> = slots
            .iter()
            .map(|slot| {
                let highlighted = nominated_position
                    .map(|np| slot_matches_position(slot, np))
                    .unwrap_or(false);
                slot_row(slot, highlighted)
            })
            .collect();

        let list: Element<RosterMessage> = if rows.is_empty() {
            empty_placeholder()
        } else {
            v_stack(
                rows,
                StackStyle {
                    gap: StackGap::None,
                    width: Length::Fill,
                    ..Default::default()
                },
            )
            .into()
        };

        let scrollable = scrollable::Scrollable::new(list)
            .id(self.scroll_id.clone())
            .on_scroll(|_| RosterMessage::UserScrolled)
            .width(Length::Fill)
            .height(Length::Fill);
        let scrollable_elem: Element<RosterMessage> = scrollable.into();

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

        let panel_elem: Element<RosterMessage> = panel.into();
        focus_ring(panel_elem, focused)
    }
}

// ---------------------------------------------------------------------------
// Row rendering helpers
// ---------------------------------------------------------------------------

fn slot_row<'a>(slot: &'a RosterSlot, highlighted: bool) -> Element<'a, RosterMessage> {
    let pos_text: Element<RosterMessage> = text(
        slot.position.display_str(),
        TextStyle {
            size: TextSize::Xs,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();
    let pos_cell: Element<RosterMessage> = frame(
        pos_text,
        BoxStyle {
            width: Length::Fixed(36.0),
            padding: Padding::new(2.0),
            ..Default::default()
        },
    )
    .into();

    let player_text = slot.player.as_ref().map(|p| p.name.as_str()).unwrap_or("—");
    let player_color = if slot.player.is_some() {
        TextColor::Default
    } else {
        TextColor::Dimmed
    };

    let name_cell: Element<RosterMessage> = text(
        player_text,
        TextStyle {
            size: TextSize::Xs,
            color: player_color,
            ..Default::default()
        },
    )
    .into();

    let row: Element<RosterMessage> = h_stack(
        vec![pos_cell, name_cell],
        StackStyle {
            gap: StackGap::Xs,
            width: Length::Fill,
            padding: Padding::new(2.0),
            ..Default::default()
        },
    )
    .into();

    if highlighted {
        let bg = Colors::Tertiary.rgba(Opacity::O20);
        iced::widget::container(row)
            .width(Length::Fill)
            .style(move |_| iced::widget::container::Style {
                background: Some(iced::Background::Color(bg)),
                ..Default::default()
            })
            .into()
    } else {
        row
    }
}

fn empty_placeholder<'a>() -> Element<'a, RosterMessage> {
    let t: Element<RosterMessage> = text(
        "No roster data",
        TextStyle {
            size: TextSize::Xs,
            color: TextColor::Dimmed,
            ..Default::default()
        },
    )
    .into();
    frame(
        t,
        BoxStyle {
            padding: Padding::new(8.0),
            ..Default::default()
        },
    )
    .into()
}

// ---------------------------------------------------------------------------
// Position matching
// ---------------------------------------------------------------------------

fn slot_matches_position(slot: &RosterSlot, nominated_str: &str) -> bool {
    if slot.position.display_str() == nominated_str {
        return true;
    }
    if let Some(nom_pos) = Position::from_str_pos(nominated_str) {
        if slot.position.is_combo_slot()
            && slot.position.accepted_positions().contains(&nom_pos)
        {
            return true;
        }
        if slot.position == Position::Utility && nom_pos.is_hitter() {
            return true;
        }
    }
    false
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
    use wyncast_baseball::draft::roster::RosteredPlayer;

    fn make_empty_slot(pos: Position) -> RosterSlot {
        RosterSlot { position: pos, player: None }
    }

    fn make_filled_slot(pos: Position, name: &str) -> RosterSlot {
        RosterSlot {
            position: pos,
            player: Some(RosteredPlayer {
                name: name.to_string(),
                price: 10,
                position: pos,
                eligible_slots: vec![],
                espn_player_id: None,
            }),
        }
    }

    #[test]
    fn exact_match_returns_true() {
        let slot = make_empty_slot(Position::FirstBase);
        assert!(slot_matches_position(&slot, "1B"));
    }

    #[test]
    fn no_match_different_position() {
        let slot = make_empty_slot(Position::FirstBase);
        assert!(!slot_matches_position(&slot, "2B"));
    }

    #[test]
    fn utility_matches_hitters() {
        let slot = make_empty_slot(Position::Utility);
        assert!(slot_matches_position(&slot, "1B"));
        assert!(slot_matches_position(&slot, "LF"));
        assert!(slot_matches_position(&slot, "C"));
    }

    #[test]
    fn utility_does_not_match_pitchers() {
        let slot = make_empty_slot(Position::Utility);
        assert!(!slot_matches_position(&slot, "SP"));
        assert!(!slot_matches_position(&slot, "RP"));
    }

    #[test]
    fn unknown_position_string_returns_false() {
        let slot = make_empty_slot(Position::FirstBase);
        assert!(!slot_matches_position(&slot, "INVALID"));
    }

    #[test]
    fn scroll_amount_up_is_negative() {
        let (_, dy) = scroll_amount(ScrollDirection::Up);
        assert!(dy < 0.0);
    }

    #[test]
    fn scroll_amount_down_is_positive() {
        let (_, dy) = scroll_amount(ScrollDirection::Down);
        assert!(dy > 0.0);
    }

    #[test]
    fn page_amounts_larger_than_line_amounts() {
        let (_, line) = scroll_amount(ScrollDirection::Down);
        let (_, page) = scroll_amount(ScrollDirection::PageDown);
        assert!(page > line);
    }

    #[test]
    fn roster_panel_constructs() {
        let _ = RosterPanel::new();
    }

    #[test]
    fn filled_slot_has_player_name() {
        let slot = make_filled_slot(Position::Catcher, "Salvador Perez");
        assert_eq!(slot.player.as_ref().unwrap().name, "Salvador Perez");
    }
}
