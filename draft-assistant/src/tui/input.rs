// Keyboard input handling and command dispatch.
//
// Translates crossterm key events into UserCommand messages sent to the
// app orchestrator, or into local ViewState mutations (e.g. tab switching,
// scroll). Full implementation in Task 17.

use crossterm::event::KeyEvent;
use tokio::sync::mpsc;

use crate::protocol::UserCommand;
use super::ViewState;

/// Handle a keyboard event.
///
/// This is a stub that will be fully implemented in Task 17 (TUI Input
/// and System Integration). For now it is a no-op.
pub async fn handle_key(
    _key_event: KeyEvent,
    _view_state: &mut ViewState,
    _cmd_tx: &mpsc::Sender<UserCommand>,
) {
    // Stub: real keybinding dispatch comes in Task 17.
}
