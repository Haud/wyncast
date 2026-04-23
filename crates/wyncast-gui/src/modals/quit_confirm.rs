use iced::Element;
use twui::ConfirmationModal;

/// Renders a quit confirmation modal overlay.
///
/// Returns `None` when `is_open` is false.
pub fn quit_confirm_modal<'a, Message: Clone + 'a>(
    on_confirm: Message,
    on_cancel: Message,
) -> Option<Element<'a, Message>> {
    ConfirmationModal::view(
        true,
        "Quit Wyncast?",
        "Are you sure you want to quit?",
        "Quit",
        "Cancel",
        on_confirm,
        on_cancel,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_confirm_modal_returns_some() {
        let result: Option<Element<'_, String>> =
            quit_confirm_modal("confirm".to_string(), "cancel".to_string());
        assert!(result.is_some());
    }
}
