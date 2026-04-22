use iced::keyboard::{Key, Modifiers};
use wyncast_app::protocol::UiUpdate;

use crate::screens::draft::DraftMessage;

#[derive(Debug, Clone)]
pub enum Message {
    UiUpdate(UiUpdate),
    KeyPressed(Key, Modifiers),
    Draft(DraftMessage),
    NoOp,
}
