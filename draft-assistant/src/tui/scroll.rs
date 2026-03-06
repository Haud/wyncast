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

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(offset: usize, content: usize, viewport: usize) -> ScrollState {
        ScrollState {
            offset,
            content_height: content,
            viewport_height: viewport,
        }
    }

    #[test]
    fn new_initializes_to_zeros() {
        let s = ScrollState::new();
        assert_eq!(s.offset, 0);
        assert_eq!(s.content_height, 0);
        assert_eq!(s.viewport_height, 0);
    }

    #[test]
    fn default_initializes_to_zeros() {
        let s = ScrollState::default();
        assert_eq!(s.offset, 0);
        assert_eq!(s.content_height, 0);
        assert_eq!(s.viewport_height, 0);
    }

    #[test]
    fn scroll_up_at_zero_stays_at_zero() {
        let mut s = state_with(0, 100, 10);
        s.scroll(ScrollDirection::Up);
        assert_eq!(s.offset, 0);
    }

    #[test]
    fn scroll_up_decrements_by_one() {
        let mut s = state_with(5, 100, 10);
        s.scroll(ScrollDirection::Up);
        assert_eq!(s.offset, 4);
    }

    #[test]
    fn scroll_down_increments_by_one() {
        let mut s = state_with(0, 100, 10);
        s.scroll(ScrollDirection::Down);
        assert_eq!(s.offset, 1);
    }

    #[test]
    fn scroll_down_clamps_at_max() {
        // max_offset = 100 - 10 = 90
        let mut s = state_with(90, 100, 10);
        s.scroll(ScrollDirection::Down);
        assert_eq!(s.offset, 90);
    }

    #[test]
    fn scroll_page_up_jumps_by_viewport() {
        let mut s = state_with(25, 100, 10);
        s.scroll(ScrollDirection::PageUp);
        assert_eq!(s.offset, 15);
    }

    #[test]
    fn scroll_page_up_clamps_at_zero() {
        let mut s = state_with(3, 100, 10);
        s.scroll(ScrollDirection::PageUp);
        assert_eq!(s.offset, 0);
    }

    #[test]
    fn scroll_page_down_jumps_by_viewport() {
        let mut s = state_with(0, 100, 10);
        s.scroll(ScrollDirection::PageDown);
        assert_eq!(s.offset, 10);
    }

    #[test]
    fn scroll_page_down_clamps_at_max() {
        // max_offset = 100 - 10 = 90
        let mut s = state_with(85, 100, 10);
        s.scroll(ScrollDirection::PageDown);
        assert_eq!(s.offset, 90);
    }

    #[test]
    fn scroll_top_goes_to_zero() {
        let mut s = state_with(50, 100, 10);
        s.scroll(ScrollDirection::Top);
        assert_eq!(s.offset, 0);
    }

    #[test]
    fn scroll_bottom_goes_to_max() {
        let mut s = state_with(0, 100, 10);
        s.scroll(ScrollDirection::Bottom);
        assert_eq!(s.offset, 90);
    }

    #[test]
    fn auto_scroll_to_bottom_sets_max_offset() {
        let mut s = state_with(0, 50, 10);
        s.auto_scroll_to_bottom();
        assert_eq!(s.offset, 40);
    }

    #[test]
    fn auto_scroll_to_bottom_when_content_fits() {
        let mut s = state_with(0, 5, 10);
        s.auto_scroll_to_bottom();
        assert_eq!(s.offset, 0);
    }

    #[test]
    fn set_viewport_updates_dimensions() {
        let mut s = ScrollState::new();
        s.set_viewport(50, 10);
        assert_eq!(s.content_height, 50);
        assert_eq!(s.viewport_height, 10);
    }

    #[test]
    fn set_viewport_clamps_offset_when_content_shrinks() {
        let mut s = state_with(80, 100, 10);
        // Shrink content so max_offset = 50 - 10 = 40
        s.set_viewport(50, 10);
        assert_eq!(s.offset, 40);
    }

    #[test]
    fn set_viewport_preserves_offset_when_still_valid() {
        let mut s = state_with(20, 100, 10);
        s.set_viewport(80, 10);
        assert_eq!(s.offset, 20);
    }

    #[test]
    fn content_fits_in_viewport_all_scrolls_stay_at_zero() {
        let mut s = state_with(0, 5, 10);
        for dir in [
            ScrollDirection::Up,
            ScrollDirection::Down,
            ScrollDirection::PageUp,
            ScrollDirection::PageDown,
            ScrollDirection::Top,
            ScrollDirection::Bottom,
        ] {
            s.scroll(dir);
            assert_eq!(s.offset, 0, "offset should be 0 for {:?}", dir);
        }
    }

    #[test]
    fn content_height_zero_all_scrolls_stay_at_zero() {
        let mut s = state_with(0, 0, 10);
        for dir in [
            ScrollDirection::Up,
            ScrollDirection::Down,
            ScrollDirection::PageUp,
            ScrollDirection::PageDown,
            ScrollDirection::Top,
            ScrollDirection::Bottom,
        ] {
            s.scroll(dir);
            assert_eq!(s.offset, 0, "offset should be 0 for {:?}", dir);
        }
    }

    #[test]
    fn content_and_viewport_both_zero() {
        let mut s = state_with(0, 0, 0);
        s.scroll(ScrollDirection::Down);
        assert_eq!(s.offset, 0);
        s.auto_scroll_to_bottom();
        assert_eq!(s.offset, 0);
    }
}
