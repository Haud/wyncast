use iced::widget::stack;
use iced::Element;

/// Stacks an optional overlay on top of a base element.
///
/// Used for modal overlays: the base is the screen content, the overlay is the modal.
/// When `overlay` is None the base element is returned unchanged.
pub fn with_overlay<'a, Message: 'a>(
    base: Element<'a, Message>,
    overlay: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    match overlay {
        Some(o) => stack![base, o].into(),
        None => base,
    }
}
