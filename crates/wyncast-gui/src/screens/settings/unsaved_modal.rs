// "Discard unsaved changes?" confirmation modal for the Settings screen.

use iced::Element;
use twui::ConfirmationModal;

use super::SettingsMessage;

pub fn view<'a>(is_open: bool) -> Option<Element<'a, SettingsMessage>> {
    ConfirmationModal::view(
        is_open,
        "Discard changes?",
        "You have unsaved changes. Discard them and exit settings?",
        "Discard",
        "Keep editing",
        SettingsMessage::DiscardConfirmed,
        SettingsMessage::DiscardCancelled,
    )
}
