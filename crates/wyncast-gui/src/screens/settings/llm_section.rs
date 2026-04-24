// LLM configuration section of the settings screen.
//
// Delegates rendering to forms::llm_form and maps messages to SettingsMessage.

use iced::Element;

use crate::forms::llm_form::LlmFormState;
use super::SettingsMessage;

pub fn view<'a>(state: &'a LlmFormState) -> Element<'a, SettingsMessage> {
    crate::forms::llm_form::view(state).map(SettingsMessage::LlmFormChanged)
}
