use iced::widget::row;
use iced::{Element, Length};

#[derive(Debug, Clone, Copy)]
pub enum SplitOrientation {
    Horizontal,
}

/// Static ratio split pane. V1: `on_resize` is unused; Phase 3.12 upgrades to draggable.
///
/// `ratio` is the fraction (0.0–1.0) allocated to the left/top pane.
/// Uses `Length::FillPortion` so both panes grow to fill the parent.
pub fn split_pane<'a, Message: 'a>(
    left: Element<'a, Message>,
    right: Element<'a, Message>,
    ratio: f32,
    _orientation: SplitOrientation,
    _on_resize: Option<fn(f32) -> Message>,
) -> Element<'a, Message> {
    let left_parts = (ratio * 100.0).round() as u16;
    let right_parts = 100_u16.saturating_sub(left_parts);

    let left_pane = iced::widget::container(left)
        .width(Length::FillPortion(left_parts))
        .height(Length::Fill);

    let right_pane = iced::widget::container(right)
        .width(Length::FillPortion(right_parts))
        .height(Length::Fill);

    row![left_pane, right_pane]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_pane_produces_element() {
        let left: Element<'_, String> = iced::widget::Space::new().into();
        let right: Element<'_, String> = iced::widget::Space::new().into();
        let _pane = split_pane(left, right, 0.65, SplitOrientation::Horizontal, None);
    }

    #[test]
    fn split_ratio_fills_correctly() {
        // 65/35 split: left_parts = 65, right_parts = 35
        let left_parts = (0.65_f32 * 100.0).round() as u16;
        let right_parts = 100_u16.saturating_sub(left_parts);
        assert_eq!(left_parts, 65);
        assert_eq!(right_parts, 35);
    }
}
