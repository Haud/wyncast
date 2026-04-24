use iced::keyboard::{Key, Modifiers};
use wyncast_app::protocol::UiUpdate;

use crate::screens::draft::DraftMessage;
use crate::screens::matchup::MatchupMessage;
use crate::screens::onboarding::OnboardingMessage;
use crate::screens::settings::SettingsMessage;

#[derive(Debug, Clone)]
pub enum Message {
    UiUpdate(UiUpdate),
    KeyPressed(Key, Modifiers),
    WindowResized(iced::Size),
    WindowMoved { x: i32, y: i32 },
    WindowClosed,
    Draft(DraftMessage),
    Matchup(MatchupMessage),
    Onboarding(OnboardingMessage),
    Settings(SettingsMessage),
    /// Toast dismissed by ID (close button or auto-expiry).
    ToastDismissed(u64),
    /// Toggle the keyboard help overlay on/off.
    HelpToggled,
    /// Periodic tick that drives spinner animation while streaming or disconnected.
    SpinnerTick,
    NoOp,
}
