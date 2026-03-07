// Modal overlay layer for draft mode (Elm Architecture).
//
// Composes the draft-mode modal overlays: PositionFilterModal and quit
// confirmation dialog. The parent renders this layer last so modals
// appear on top of all other content.

pub mod position_filter;

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::tui::confirm_dialog::{ConfirmDialog, ConfirmMessage, ConfirmResult};
use crate::tui::subscription::Subscription;
use crate::tui::subscription::keybinding::KeybindManager;
use position_filter::{PositionFilterModal, PositionFilterModalAction, PositionFilterModalMessage};

// ---------------------------------------------------------------------------
// Action
// ---------------------------------------------------------------------------

/// Actions returned by [`ModalLayer::update`] for the parent to handle.
#[derive(Debug, Clone, PartialEq)]
pub enum ModalLayerAction {
    PositionFilter(PositionFilterModalAction),
    QuitConfirm(ConfirmResult),
}

// ---------------------------------------------------------------------------
// Message
// ---------------------------------------------------------------------------

/// Messages that drive the modal layer.
#[derive(Debug, Clone)]
pub enum ModalLayerMessage {
    PositionFilter(PositionFilterModalMessage),
    QuitConfirm(ConfirmMessage),
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

/// Mid-level component that owns all draft-mode modal overlays.
///
/// Fields are `pub` because the parent needs direct access for input routing
/// (checking `open` state and calling `key_to_message` on children).
#[derive(Debug, Clone)]
pub struct ModalLayer {
    pub position_filter: PositionFilterModal,
    pub quit_confirm: ConfirmDialog,
}

impl ModalLayer {
    pub fn new() -> Self {
        Self {
            position_filter: PositionFilterModal::default(),
            quit_confirm: ConfirmDialog::quit(),
        }
    }

    /// Returns `true` if any modal is currently intercepting input.
    pub fn has_active_modal(&self) -> bool {
        self.position_filter.open || self.quit_confirm.open
    }

    /// Declare keybindings for the subscription system.
    ///
    /// Only the open modal (if any) subscribes — quit confirm is checked first
    /// (it has higher visual precedence), then position filter. Both are
    /// mutually exclusive in normal flow, but the batch order encodes priority.
    pub fn subscription(&self, kb: &mut KeybindManager) -> Subscription<ModalLayerMessage> {
        let quit_sub = self
            .quit_confirm
            .subscription(kb)
            .map(ModalLayerMessage::QuitConfirm);

        let pos_sub = self
            .position_filter
            .subscription(kb)
            .map(ModalLayerMessage::PositionFilter);

        Subscription::batch([quit_sub, pos_sub])
    }

    /// Process a message and return an optional action for the parent.
    pub fn update(&mut self, msg: ModalLayerMessage) -> Option<ModalLayerAction> {
        match msg {
            ModalLayerMessage::PositionFilter(m) => {
                self.position_filter.update(m).map(ModalLayerAction::PositionFilter)
            }
            ModalLayerMessage::QuitConfirm(m) => {
                self.quit_confirm.update(m).map(ModalLayerAction::QuitConfirm)
            }
        }
    }

    /// Render all open modals. Position filter renders first; quit confirm
    /// renders last (on top).
    pub fn view(&self, frame: &mut Frame, area: Rect) {
        if self.position_filter.open {
            self.position_filter.view(frame, area);
        }
        if self.quit_confirm.open {
            self.quit_confirm.view(frame, area);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::pick::Position;

    #[test]
    fn new_starts_with_no_active_modal() {
        let layer = ModalLayer::new();
        assert!(!layer.has_active_modal());
        assert!(!layer.position_filter.open);
        assert!(!layer.quit_confirm.open);
    }

    #[test]
    fn has_active_modal_position_filter() {
        let mut layer = ModalLayer::new();
        layer.position_filter.open = true;
        assert!(layer.has_active_modal());
    }

    #[test]
    fn has_active_modal_quit_confirm() {
        let mut layer = ModalLayer::new();
        layer.quit_confirm.open = true;
        assert!(layer.has_active_modal());
    }

    #[test]
    fn update_position_filter_forwards() {
        let mut layer = ModalLayer::new();
        let msg = ModalLayerMessage::PositionFilter(PositionFilterModalMessage::Open {
            current_filter: None,
        });
        let action = layer.update(msg);
        assert!(action.is_none()); // Open returns None
        assert!(layer.position_filter.open);

        let msg = ModalLayerMessage::PositionFilter(PositionFilterModalMessage::Close);
        let action = layer.update(msg);
        assert_eq!(
            action,
            Some(ModalLayerAction::PositionFilter(
                PositionFilterModalAction::Cancelled
            ))
        );
        assert!(!layer.position_filter.open);
    }

    #[test]
    fn update_quit_confirm_forwards() {
        let mut layer = ModalLayer::new();
        let msg = ModalLayerMessage::QuitConfirm(ConfirmMessage::Open);
        let action = layer.update(msg);
        assert!(action.is_none());
        assert!(layer.quit_confirm.open);

        let msg = ModalLayerMessage::QuitConfirm(ConfirmMessage::Cancel);
        let action = layer.update(msg);
        assert_eq!(
            action,
            Some(ModalLayerAction::QuitConfirm(ConfirmResult::Cancelled))
        );
        assert!(!layer.quit_confirm.open);
    }

    #[test]
    fn update_quit_confirm_confirmed() {
        let mut layer = ModalLayer::new();
        layer.quit_confirm.open = true;
        let msg = ModalLayerMessage::QuitConfirm(ConfirmMessage::Confirm('y'));
        let action = layer.update(msg);
        assert_eq!(
            action,
            Some(ModalLayerAction::QuitConfirm(ConfirmResult::Confirmed('y')))
        );
    }

    #[test]
    fn update_position_filter_selected() {
        let mut layer = ModalLayer::new();
        layer.update(ModalLayerMessage::PositionFilter(
            PositionFilterModalMessage::Open {
                current_filter: None,
            },
        ));
        // Move down to Catcher then confirm
        layer.update(ModalLayerMessage::PositionFilter(
            PositionFilterModalMessage::MoveDown,
        ));
        let action = layer.update(ModalLayerMessage::PositionFilter(
            PositionFilterModalMessage::Confirm,
        ));
        assert_eq!(
            action,
            Some(ModalLayerAction::PositionFilter(
                PositionFilterModalAction::Selected(Some(Position::Catcher))
            ))
        );
    }

    #[test]
    fn view_does_not_panic_with_both_open() {
        let mut layer = ModalLayer::new();
        layer.position_filter.open = true;
        layer.quit_confirm.open = true;
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| layer.view(frame, frame.area()))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_when_closed() {
        let layer = ModalLayer::new();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| layer.view(frame, frame.area()))
            .unwrap();
    }
}
