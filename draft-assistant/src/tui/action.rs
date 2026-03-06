use crate::protocol::UserCommand;

/// Returned by component update() to communicate intent upward.
/// Components return `Option<Action>` — `None` means the component handled
/// the message internally with no upward effect.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Send a command to the app backend
    Command(UserCommand),
    /// Request the TUI event loop to exit
    Quit,
}
