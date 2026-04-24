// Strategy configuration section of the settings screen.
//
// Delegates rendering to forms::strategy_form and maps messages to SettingsMessage.

use iced::Element;

use crate::forms::strategy_form::StrategyFormState;
use super::SettingsMessage;

pub fn view<'a>(state: &'a StrategyFormState) -> Element<'a, SettingsMessage> {
    crate::forms::strategy_form::view(state).map(SettingsMessage::StrategyFormChanged)
}
