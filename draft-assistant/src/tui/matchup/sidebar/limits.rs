// Limits panel: GS limit, acquisitions, days remaining, and games today.

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::tui::action::Action;
use crate::tui::scroll::{ScrollDirection, ScrollState};

// ---------------------------------------------------------------------------
// LimitsData
// ---------------------------------------------------------------------------

/// Bundled data for the limits panel view, avoiding too many function arguments.
pub struct LimitsData {
    pub gs_used: u8,
    pub gs_limit: u8,
    pub acq_used: u8,
    pub acq_limit: u8,
    pub days_remaining: usize,
    pub games_today: usize,
    pub total_active: usize,
}

// ---------------------------------------------------------------------------
// LimitsPanel
// ---------------------------------------------------------------------------

/// Limits panel showing GS usage, acquisitions, days remaining, and games today.
pub struct LimitsPanel {
    scroll: ScrollState,
}

/// Message type for the limits panel.
#[derive(Debug, Clone)]
pub enum LimitsPanelMessage {
    Scroll(ScrollDirection),
}

impl LimitsPanel {
    pub fn new() -> Self {
        Self {
            scroll: ScrollState::new(),
        }
    }

    pub fn update(&mut self, msg: LimitsPanelMessage) -> Option<Action> {
        match msg {
            LimitsPanelMessage::Scroll(dir) => {
                self.scroll.scroll(dir, 10);
                None
            }
        }
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll.offset()
    }

    pub fn view(
        &self,
        _frame: &mut Frame,
        _area: Rect,
        _data: &LimitsData,
        _focused: bool,
    ) {
    }
}
