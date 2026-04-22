use iced::border::Radius;
use iced::widget::container;
use iced::{Border, Element};
use twui::Colors;

/// Wraps a child element with a colored border ring when focused.
///
/// Focused: 2px teal border (Colors::Focus). Unfocused: transparent border (no visual change).
pub fn focus_ring<'a, Message: 'a>(
    child: impl Into<Element<'a, Message>>,
    focused: bool,
) -> Element<'a, Message> {
    let border_color = if focused {
        Colors::Focus.rgb()
    } else {
        iced::Color::TRANSPARENT
    };

    container(child)
        .style(move |_| container::Style {
            border: Border {
                color: border_color,
                width: 2.0,
                radius: Radius::new(6.0),
            },
            ..Default::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_ring_produces_element() {
        let child: Element<'_, String> = iced::widget::Space::new().into();
        let _ring = focus_ring(child, true);
    }

    #[test]
    fn unfocused_focus_ring_produces_element() {
        let child: Element<'_, String> = iced::widget::Space::new().into();
        let _ring = focus_ring(child, false);
    }
}
