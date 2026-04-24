use iced::keyboard::{Key, Modifiers};
use wyncast_app::protocol::UiUpdate;

use crate::screens::draft::DraftMessage;
use crate::screens::matchup::MatchupMessage;
use crate::screens::onboarding::OnboardingMessage;

#[derive(Debug, Clone)]
pub enum Message {
    UiUpdate(UiUpdate),
    KeyPressed(Key, Modifiers),
    WindowResized(iced::Size),
    Draft(DraftMessage),
    Matchup(MatchupMessage),
    Onboarding(OnboardingMessage),
    /// Periodic tick that drives spinner animation while disconnected.
    SpinnerTick,
    NoOp,
}
