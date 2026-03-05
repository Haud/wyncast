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

/// Scroll directions for TUI components.
/// Extends the protocol ScrollDirection with Home/End support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
    PageUp,
    PageDown,
    Top,
    Bottom,
}

/// Reusable scroll state for any scrollable panel.
#[derive(Debug, Clone)]
pub struct ScrollState {
    pub offset: usize,
    pub content_height: usize,
    pub viewport_height: usize,
}

impl ScrollState {
    pub fn new() -> Self {
        Self {
            offset: 0,
            content_height: 0,
            viewport_height: 0,
        }
    }

    /// Apply a scroll direction, clamping to valid range.
    pub fn scroll(&mut self, direction: ScrollDirection) {
        let max_offset = self.content_height.saturating_sub(self.viewport_height);
        self.offset = match direction {
            ScrollDirection::Up => self.offset.saturating_sub(1),
            ScrollDirection::Down => (self.offset + 1).min(max_offset),
            ScrollDirection::PageUp => self.offset.saturating_sub(self.viewport_height),
            ScrollDirection::PageDown => (self.offset + self.viewport_height).min(max_offset),
            ScrollDirection::Top => 0,
            ScrollDirection::Bottom => max_offset,
        };
    }

    /// Auto-scroll to bottom (used during LLM streaming).
    pub fn auto_scroll_to_bottom(&mut self) {
        let max_offset = self.content_height.saturating_sub(self.viewport_height);
        self.offset = max_offset;
    }

    /// Update dimensions (called during view/render).
    pub fn set_viewport(&mut self, content_lines: usize, viewport_lines: usize) {
        self.content_height = content_lines;
        self.viewport_height = viewport_lines;
        // Clamp offset if content shrank
        let max_offset = self.content_height.saturating_sub(self.viewport_height);
        if self.offset > max_offset {
            self.offset = max_offset;
        }
    }
}

impl Default for ScrollState {
    fn default() -> Self {
        Self::new()
    }
}
