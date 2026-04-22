use iced::border::Radius;
use iced::widget::{text_input, Id};
use iced::{Background, Border, Element, Length, Padding, Pixels};
use twui::Colors;
use iced_fonts::LUCIDE_FONT;

/// Styled text input for filtering, with a known widget `Id` for programmatic focus.
///
/// Uses iced's raw `text_input` (allowed within this widget file) with styling
/// matching twui's `text_field` aesthetic.  The caller stores the `Id` and
/// fires `iced::widget::operation::focus(id)` when the user presses `/`.
///
/// Flagged for upstream to twui once twui exposes widget IDs.
pub fn filter_input<'a, Message: Clone + 'a>(
    value: &'a str,
    on_change: impl Fn(String) -> Message + 'a,
    id: Id,
    placeholder: &'a str,
) -> Element<'a, Message> {
    text_input(placeholder, value)
        .on_input(on_change)
        .id(id)
        .width(Length::Fill)
        .padding(Padding {
            top: 8.0,
            bottom: 8.0,
            left: 10.0,
            right: 10.0,
        })
        .icon(text_input::Icon {
            font: LUCIDE_FONT,
            code_point: twui::Icons::Search.code_point(),
            size: Some(Pixels(14.0)),
            spacing: 6.0,
            side: text_input::Side::Left,
        })
        .style(|_theme, status| {
            let border_color = match status {
                text_input::Status::Focused { .. } => Colors::Focus.rgb(),
                text_input::Status::Hovered => Colors::BorderDefault.rgb(),
                _ => Colors::BorderDefault.rgb(),
            };

            text_input::Style {
                background: Background::Color(Colors::BgInput.rgb()),
                border: Border {
                    color: border_color,
                    width: 1.5,
                    radius: Radius::new(6.0),
                },
                icon: Colors::TextTertiary.rgb(),
                placeholder: Colors::TextTertiary.rgb(),
                value: Colors::TextPrimary.rgb(),
                selection: Colors::Focus.rgba(twui::Opacity::O30),
            }
        })
        .into()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_input_produces_element() {
        let id = Id::unique();
        let _elem: Element<'_, String> =
            filter_input("", |s| s, id, "Filter players...");
    }

    #[test]
    fn filter_input_with_value_produces_element() {
        let id = Id::unique();
        let _elem: Element<'_, String> =
            filter_input("mike", |s| s, id, "Filter...");
    }
}
