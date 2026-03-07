// Position filter modal component (Elm Architecture).
//
// A centered modal overlay that lets the user select a position filter for the
// Available Players tab.  Owns its open/closed state, incremental search text,
// and highlighted selection index.
//
// Messages flow through `key_to_message()` -> `update()` which returns an
// optional `PositionFilterModalAction` for the parent to act on.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::draft::pick::Position;
use crate::tui::text_input::TextInput;
use crate::tui::subscription::{
    Subscription, SubscriptionId,
    keybinding::{
        exact, KeyBindingRecipe, KeybindHint, KeybindManager, KeyTrigger, PRIORITY_MODAL,
    },
};

// ---------------------------------------------------------------------------
// Action
// ---------------------------------------------------------------------------

/// Actions returned by `update()` for the parent to handle.
#[derive(Debug, Clone, PartialEq)]
pub enum PositionFilterModalAction {
    /// The user confirmed a selection. Parent should send SetPositionFilter to
    /// AvailablePanel.
    Selected(Option<Position>),
    /// The user cancelled. Parent needs no action beyond acknowledging close.
    Cancelled,
}

// ---------------------------------------------------------------------------
// Message
// ---------------------------------------------------------------------------

/// Messages that drive the position filter modal state machine.
#[derive(Debug, Clone)]
pub enum PositionFilterModalMessage {
    /// Open the modal, pre-selecting the option matching `current_filter`.
    Open { current_filter: Option<Position> },
    /// Cancel (Esc) -- close without applying.
    Close,
    /// Apply the selected option, close.
    Confirm,
    /// Move selection up.
    MoveUp,
    /// Move selection down.
    MoveDown,
    /// Forward a key event to the search TextInput.
    SearchKey(KeyEvent),
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

/// Width of the modal dialog.
const MODAL_WIDTH: u16 = 30;

/// State for the position filter modal overlay.
#[derive(Debug, Clone)]
pub struct PositionFilterModal {
    /// Whether the modal is currently visible.
    pub open: bool,
    /// Incremental search text typed by the user while the modal is open.
    search_text: TextInput,
    /// Index into the *filtered* list of options that is currently highlighted.
    selected_index: usize,
    sub_id: SubscriptionId,
}

impl Default for PositionFilterModal {
    fn default() -> Self {
        Self {
            open: false,
            search_text: TextInput::default(),
            selected_index: 0,
            sub_id: SubscriptionId::unique(),
        }
    }
}

impl PositionFilterModal {
    /// The full ordered list of selectable options (None = "ALL").
    pub const OPTIONS: &'static [Option<Position>] = &[
        None,
        Some(Position::Catcher),
        Some(Position::FirstBase),
        Some(Position::SecondBase),
        Some(Position::ThirdBase),
        Some(Position::ShortStop),
        Some(Position::LeftField),
        Some(Position::CenterField),
        Some(Position::RightField),
        Some(Position::Utility),
        Some(Position::StartingPitcher),
        Some(Position::ReliefPitcher),
    ];

    /// Return the display label for an option.
    pub fn option_label(opt: Option<Position>) -> &'static str {
        match opt {
            None => "ALL",
            Some(p) => p.display_str(),
        }
    }

    /// Return the subset of options that match the current search text
    /// (case-insensitive substring match).
    pub fn filtered_options(&self) -> Vec<Option<Position>> {
        let search = self.search_text.value().to_uppercase();
        Self::OPTIONS
            .iter()
            .copied()
            .filter(|opt| {
                let label = Self::option_label(*opt);
                label.contains(search.as_str())
            })
            .collect()
    }

    // -- Elm Architecture API ------------------------------------------------

    /// Convert a key event into a message when the modal is open.
    ///
    /// Returns `None` if the modal is closed or the key is not handled.
    pub fn key_to_message(&self, key: KeyEvent) -> Option<PositionFilterModalMessage> {
        if !self.open {
            return None;
        }
        match key.code {
            KeyCode::Esc => Some(PositionFilterModalMessage::Close),
            KeyCode::Enter => Some(PositionFilterModalMessage::Confirm),
            KeyCode::Up => Some(PositionFilterModalMessage::MoveUp),
            KeyCode::Down => Some(PositionFilterModalMessage::MoveDown),
            _ => {
                // Only forward if TextInput would handle this key
                if TextInput::key_to_message(&key).is_some() {
                    Some(PositionFilterModalMessage::SearchKey(key))
                } else {
                    None
                }
            }
        }
    }

    /// Declare keybindings for the subscription system.
    ///
    /// Returns a capturing `Subscription<PositionFilterModalMessage>` at
    /// `PRIORITY_MODAL` when the modal is open, or `Subscription::none()` when
    /// closed.
    pub fn subscription(
        &self,
        kb: &mut KeybindManager,
    ) -> Subscription<PositionFilterModalMessage> {
        if !self.open {
            return Subscription::none();
        }

        let recipe = KeyBindingRecipe::new(self.sub_id)
            .priority(PRIORITY_MODAL)
            .capture()
            .bind(
                exact(KeyCode::Esc),
                |_| PositionFilterModalMessage::Close,
                KeybindHint::new("Esc", "Cancel"),
            )
            .bind(
                exact(KeyCode::Enter),
                |_| PositionFilterModalMessage::Confirm,
                KeybindHint::new("Enter", "Select"),
            )
            .bind(
                exact(KeyCode::Up),
                |_| PositionFilterModalMessage::MoveUp,
                KeybindHint::new("↑", "Up"),
            )
            .bind(
                exact(KeyCode::Down),
                |_| PositionFilterModalMessage::MoveDown,
                KeybindHint::new("↓", "Down"),
            )
            .bind(
                KeyTrigger::AnyChar,
                |k| PositionFilterModalMessage::SearchKey(k),
                KeybindHint::new("a-z", "Search"),
            );

        kb.subscribe(recipe)
    }

    /// Process a message and return an optional action for the parent.
    pub fn update(
        &mut self,
        msg: PositionFilterModalMessage,
    ) -> Option<PositionFilterModalAction> {
        match msg {
            PositionFilterModalMessage::Open { current_filter } => {
                self.open = true;
                self.search_text.clear();
                let idx = Self::OPTIONS
                    .iter()
                    .position(|opt| *opt == current_filter)
                    .unwrap_or(0);
                self.selected_index = idx;
                None
            }
            PositionFilterModalMessage::Close => {
                self.open = false;
                self.search_text.clear();
                Some(PositionFilterModalAction::Cancelled)
            }
            PositionFilterModalMessage::Confirm => {
                let options = self.filtered_options();
                let action = if !options.is_empty() {
                    let idx = self.selected_index.min(options.len() - 1);
                    Some(PositionFilterModalAction::Selected(options[idx]))
                } else {
                    Some(PositionFilterModalAction::Cancelled)
                };
                self.open = false;
                self.search_text.clear();
                action
            }
            PositionFilterModalMessage::MoveUp => {
                self.selected_index = self.selected_index.saturating_sub(1);
                None
            }
            PositionFilterModalMessage::MoveDown => {
                let option_count = self.filtered_options().len();
                if option_count > 0 {
                    self.selected_index = (self.selected_index + 1).min(option_count - 1);
                }
                None
            }
            PositionFilterModalMessage::SearchKey(key_event) => {
                let modifies_text = matches!(
                    key_event.code,
                    KeyCode::Backspace | KeyCode::Delete | KeyCode::Char(_)
                );
                if let Some(msg) = TextInput::key_to_message(&key_event) {
                    self.search_text.update(msg);
                }
                if modifies_text {
                    self.selected_index = 0;
                }
                None
            }
        }
    }

    /// Render the modal overlay. Only draws when `self.open` is true.
    pub fn view(&self, frame: &mut Frame, area: Rect) {
        if !self.open {
            return;
        }

        let options = self.filtered_options();

        // Height: border(2) + search bar(1) + items (up to 14, min 1)
        let item_count = options.len().max(1).min(14) as u16;
        let modal_height = 2 + 1 + item_count;
        let modal_area = centered_rect(MODAL_WIDTH, modal_height, area);

        // Clear background behind the modal
        frame.render_widget(Clear, modal_area);

        // Outer block
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(Span::styled(
                " Filter by Position ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));

        let inner_area = block.inner(modal_area);
        frame.render_widget(block, modal_area);

        if inner_area.height == 0 || inner_area.width == 0 {
            return;
        }

        // Split inner area: search bar (1 row) + list (remaining)
        let inner_chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(inner_area);

        let search_area = inner_chunks[0];
        let list_area = inner_chunks[1];

        // --- Search bar ---
        let search_spans = vec![
            Span::styled(
                ">",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                self.search_text.value(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("▎", Style::default().fg(Color::Cyan)),
        ];
        let search_paragraph = Paragraph::new(Line::from(search_spans));
        frame.render_widget(search_paragraph, search_area);

        // --- Options list ---
        if list_area.height == 0 {
            return;
        }

        let items: Vec<ListItem> = options
            .iter()
            .map(|opt| {
                let label = Self::option_label(*opt);
                ListItem::new(Line::from(format!(" {} ", label)))
            })
            .collect();

        let list = List::new(items)
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");

        // Clamp selected_index to the filtered list length
        let clamped_index = if options.is_empty() {
            0
        } else {
            self.selected_index.min(options.len() - 1)
        };

        let mut list_state = ListState::default();
        if !options.is_empty() {
            list_state.select(Some(clamped_index));
        }

        frame.render_stateful_widget(list, list_area, &mut list_state);
    }
}

/// Compute a centered rectangle of the given size within `area`.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let clamped_width = width.min(area.width);
    let clamped_height = height.min(area.height);

    let vertical = Layout::vertical([Constraint::Length(clamped_height)])
        .flex(Flex::Center)
        .split(area);

    let horizontal = Layout::horizontal([Constraint::Length(clamped_width)])
        .flex(Flex::Center)
        .split(vertical[0]);

    horizontal[0]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_rect_is_centered() {
        let area = Rect::new(0, 0, 80, 24);
        let result = centered_rect(MODAL_WIDTH, 10, area);
        assert_eq!(result.width, MODAL_WIDTH);
        assert_eq!(result.height, 10);
        let center_x = area.width / 2;
        let result_center_x = result.x + result.width / 2;
        assert!(
            (result_center_x as i32 - center_x as i32).unsigned_abs() <= 1,
            "Modal should be horizontally centered"
        );
    }

    #[test]
    fn centered_rect_clamps_to_small_area() {
        let area = Rect::new(0, 0, 10, 3);
        let result = centered_rect(MODAL_WIDTH, 10, area);
        assert!(result.width <= area.width);
        assert!(result.height <= area.height);
    }

    #[test]
    fn centered_rect_handles_zero_area() {
        let area = Rect::new(0, 0, 0, 0);
        let result = centered_rect(MODAL_WIDTH, 10, area);
        assert_eq!(result.width, 0);
        assert_eq!(result.height, 0);
    }

    #[test]
    fn view_does_not_panic_when_modal_open() {
        let mut modal = PositionFilterModal::default();
        modal.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| modal.view(frame, frame.area()))
            .unwrap();
    }

    #[test]
    fn view_does_not_render_when_closed() {
        let modal = PositionFilterModal::default();
        assert!(!modal.open);
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| modal.view(frame, frame.area()))
            .unwrap();
    }

    #[test]
    fn view_does_not_panic_on_small_terminal() {
        let mut modal = PositionFilterModal::default();
        modal.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });
        let backend = ratatui::backend::TestBackend::new(10, 5);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| modal.view(frame, frame.area()))
            .unwrap();
    }

    #[test]
    fn open_pre_selects_matching_option() {
        let mut modal = PositionFilterModal::default();
        modal.update(PositionFilterModalMessage::Open {
            current_filter: Some(Position::StartingPitcher),
        });
        assert!(modal.open);
        let expected = PositionFilterModal::OPTIONS
            .iter()
            .position(|opt| *opt == Some(Position::StartingPitcher))
            .unwrap();
        assert_eq!(modal.selected_index, expected);
    }

    #[test]
    fn open_defaults_to_zero_when_no_match() {
        let mut modal = PositionFilterModal::default();
        modal.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });
        assert_eq!(modal.selected_index, 0);
    }

    #[test]
    fn close_returns_cancelled() {
        let mut modal = PositionFilterModal::default();
        modal.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });
        let action = modal.update(PositionFilterModalMessage::Close);
        assert_eq!(action, Some(PositionFilterModalAction::Cancelled));
        assert!(!modal.open);
    }

    #[test]
    fn confirm_returns_selected() {
        let mut modal = PositionFilterModal::default();
        modal.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });
        // selected_index 0 = ALL = None
        let action = modal.update(PositionFilterModalMessage::Confirm);
        assert_eq!(action, Some(PositionFilterModalAction::Selected(None)));
        assert!(!modal.open);
    }

    #[test]
    fn confirm_selects_correct_option() {
        let mut modal = PositionFilterModal::default();
        modal.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });
        // Move down to index 1 = Catcher
        modal.update(PositionFilterModalMessage::MoveDown);
        let action = modal.update(PositionFilterModalMessage::Confirm);
        assert_eq!(
            action,
            Some(PositionFilterModalAction::Selected(Some(Position::Catcher)))
        );
    }

    #[test]
    fn move_up_saturates_at_zero() {
        let mut modal = PositionFilterModal::default();
        modal.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });
        modal.update(PositionFilterModalMessage::MoveUp);
        assert_eq!(modal.selected_index, 0);
    }

    #[test]
    fn move_down_saturates_at_max() {
        let mut modal = PositionFilterModal::default();
        modal.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });
        let max_idx = PositionFilterModal::OPTIONS.len() - 1;
        for _ in 0..20 {
            modal.update(PositionFilterModalMessage::MoveDown);
        }
        assert_eq!(modal.selected_index, max_idx);
    }

    #[test]
    fn search_key_resets_selection_on_text_modification() {
        let mut modal = PositionFilterModal::default();
        modal.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });
        // Move to index 3
        modal.update(PositionFilterModalMessage::MoveDown);
        modal.update(PositionFilterModalMessage::MoveDown);
        modal.update(PositionFilterModalMessage::MoveDown);
        assert_eq!(modal.selected_index, 3);

        // Type 's' — should reset to 0
        let key = KeyEvent::new(KeyCode::Char('s'), crossterm::event::KeyModifiers::NONE);
        modal.update(PositionFilterModalMessage::SearchKey(key));
        assert_eq!(modal.selected_index, 0);
    }

    #[test]
    fn key_to_message_returns_none_when_closed() {
        let modal = PositionFilterModal::default();
        let key = KeyEvent::new(KeyCode::Esc, crossterm::event::KeyModifiers::NONE);
        assert!(modal.key_to_message(key).is_none());
    }

    #[test]
    fn key_to_message_maps_esc_to_close() {
        let mut modal = PositionFilterModal::default();
        modal.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });
        let key = KeyEvent::new(KeyCode::Esc, crossterm::event::KeyModifiers::NONE);
        assert!(matches!(
            modal.key_to_message(key),
            Some(PositionFilterModalMessage::Close)
        ));
    }

    #[test]
    fn key_to_message_maps_enter_to_confirm() {
        let mut modal = PositionFilterModal::default();
        modal.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });
        let key = KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE);
        assert!(matches!(
            modal.key_to_message(key),
            Some(PositionFilterModalMessage::Confirm)
        ));
    }

    #[test]
    fn filtered_options_narrows_list() {
        let mut modal = PositionFilterModal::default();
        modal.update(PositionFilterModalMessage::Open {
            current_filter: None,
        });
        let key = KeyEvent::new(KeyCode::Char('S'), crossterm::event::KeyModifiers::NONE);
        modal.update(PositionFilterModalMessage::SearchKey(key));
        let key2 = KeyEvent::new(KeyCode::Char('P'), crossterm::event::KeyModifiers::NONE);
        modal.update(PositionFilterModalMessage::SearchKey(key2));
        let options = modal.filtered_options();
        assert_eq!(options, vec![Some(Position::StartingPitcher)]);
    }
}
