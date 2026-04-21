use iced::keyboard::{Key, Modifiers};
use wyncast_app::protocol::UiUpdate;

#[derive(Debug, Clone)]
pub enum Message {
    UiUpdate(UiUpdate),
    KeyPressed(Key, Modifiers),
    NoOp,
}
