use std::cell::Cell;

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
///
/// Stores only the scroll offset. Content and viewport dimensions are passed
/// in at call sites — `scroll()` does NOT clamp against content bounds (the
/// offset may temporarily exceed the valid range). Use `clamped_offset()` at
/// render time to obtain a safe value.
///
/// The offset is stored in a `Cell<usize>` so that `clamped_offset()` can
/// normalize the stored value at render time (when actual dimensions are known),
/// even though it only takes `&self`. This ensures that after rendering, any
/// sentinel values (like `usize::MAX` from `scroll_to_end()`) are replaced with
/// the real clamped position, so subsequent scroll operations work correctly.
#[derive(Debug, Clone)]
pub struct ScrollState {
    offset: Cell<usize>,
}

impl ScrollState {
    pub fn new() -> Self {
        Self { offset: Cell::new(0) }
    }

    /// Current raw offset (may exceed valid range until clamped).
    pub fn offset(&self) -> usize {
        self.offset.get()
    }

    /// Reset offset to 0.
    pub fn reset(&mut self) {
        self.offset.set(0);
    }

    /// Apply a scroll direction.
    ///
    /// `viewport_height` is needed for PageUp / PageDown step size.
    /// No clamping is performed — use `clamped_offset()` at render time.
    pub fn scroll(&mut self, direction: ScrollDirection, viewport_height: usize) {
        let current = self.offset.get();
        self.offset.set(match direction {
            ScrollDirection::Up => current.saturating_sub(1),
            ScrollDirection::Down => current.saturating_add(1),
            ScrollDirection::PageUp => current.saturating_sub(viewport_height),
            ScrollDirection::PageDown => current.saturating_add(viewport_height),
            ScrollDirection::Top => 0,
            ScrollDirection::Bottom => usize::MAX,
        });
    }

    /// Jump to the end. The actual max offset will be resolved by
    /// `clamped_offset()` at render time.
    pub fn scroll_to_end(&mut self) {
        self.offset.set(usize::MAX);
    }

    /// Clamp offset to valid range given current content/viewport dimensions.
    /// Use this in view() to safely read the offset.
    ///
    /// Also normalizes the stored offset to the clamped value, so that
    /// subsequent scroll operations start from a valid position rather than
    /// from a sentinel value like `usize::MAX`.
    pub fn clamped_offset(&self, content_height: usize, viewport_height: usize) -> usize {
        let max_offset = content_height.saturating_sub(viewport_height);
        let clamped = self.offset.get().min(max_offset);
        self.offset.set(clamped);
        clamped
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

    #[test]
    fn new_initializes_offset_to_zero() {
        let s = ScrollState::new();
        assert_eq!(s.offset(), 0);
    }

    #[test]
    fn default_initializes_offset_to_zero() {
        let s = ScrollState::default();
        assert_eq!(s.offset(), 0);
    }

    #[test]
    fn reset_sets_offset_to_zero() {
        let mut s = ScrollState::new();
        s.offset.set(42);
        s.reset();
        assert_eq!(s.offset(), 0);
    }

    #[test]
    fn scroll_up_at_zero_stays_at_zero() {
        let mut s = ScrollState::new();
        s.scroll(ScrollDirection::Up, 10);
        assert_eq!(s.offset(), 0);
    }

    #[test]
    fn scroll_up_decrements_by_one() {
        let mut s = ScrollState::new();
        s.offset.set(5);
        s.scroll(ScrollDirection::Up, 10);
        assert_eq!(s.offset(), 4);
    }

    #[test]
    fn scroll_down_increments_by_one() {
        let mut s = ScrollState::new();
        s.scroll(ScrollDirection::Down, 10);
        assert_eq!(s.offset(), 1);
    }

    #[test]
    fn scroll_down_does_not_clamp() {
        // No clamping in scroll(); clamped_offset() handles it at render time.
        let mut s = ScrollState::new();
        s.offset.set(90);
        s.scroll(ScrollDirection::Down, 10);
        assert_eq!(s.offset(), 91);
    }

    #[test]
    fn scroll_page_up_jumps_by_viewport() {
        let mut s = ScrollState::new();
        s.offset.set(25);
        s.scroll(ScrollDirection::PageUp, 10);
        assert_eq!(s.offset(), 15);
    }

    #[test]
    fn scroll_page_up_clamps_at_zero() {
        let mut s = ScrollState::new();
        s.offset.set(3);
        s.scroll(ScrollDirection::PageUp, 10);
        assert_eq!(s.offset(), 0);
    }

    #[test]
    fn scroll_page_down_jumps_by_viewport() {
        let mut s = ScrollState::new();
        s.scroll(ScrollDirection::PageDown, 10);
        assert_eq!(s.offset(), 10);
    }

    #[test]
    fn scroll_page_down_does_not_clamp() {
        let mut s = ScrollState::new();
        s.offset.set(85);
        s.scroll(ScrollDirection::PageDown, 10);
        assert_eq!(s.offset(), 95);
    }

    #[test]
    fn scroll_top_goes_to_zero() {
        let mut s = ScrollState::new();
        s.offset.set(50);
        s.scroll(ScrollDirection::Top, 10);
        assert_eq!(s.offset(), 0);
    }

    #[test]
    fn scroll_bottom_goes_to_max() {
        let mut s = ScrollState::new();
        s.scroll(ScrollDirection::Bottom, 10);
        assert_eq!(s.offset(), usize::MAX);
    }

    #[test]
    fn scroll_to_end_sets_max() {
        let mut s = ScrollState::new();
        s.scroll_to_end();
        assert_eq!(s.offset(), usize::MAX);
    }

    #[test]
    fn clamped_offset_within_bounds() {
        let s = ScrollState::new();
        s.offset.set(5);
        assert_eq!(s.clamped_offset(100, 10), 5);
    }

    #[test]
    fn clamped_offset_exceeding_bounds() {
        let s = ScrollState::new();
        s.offset.set(95);
        // max_offset = 100 - 10 = 90
        assert_eq!(s.clamped_offset(100, 10), 90);
    }

    #[test]
    fn clamped_offset_at_usize_max() {
        let mut s = ScrollState::new();
        s.scroll_to_end();
        assert_eq!(s.clamped_offset(100, 10), 90);
    }

    #[test]
    fn clamped_offset_content_fits_in_viewport() {
        let s = ScrollState::new();
        s.offset.set(50);
        // content < viewport => max_offset = 0
        assert_eq!(s.clamped_offset(5, 10), 0);
    }

    #[test]
    fn clamped_offset_zero_content() {
        let s = ScrollState::new();
        assert_eq!(s.clamped_offset(0, 10), 0);
    }

    #[test]
    fn clamped_offset_zero_content_and_viewport() {
        let s = ScrollState::new();
        assert_eq!(s.clamped_offset(0, 0), 0);
    }

    #[test]
    fn scroll_with_zero_viewport() {
        let mut s = ScrollState::new();
        // PageUp/PageDown with viewport 0 are effectively no-ops on offset
        s.offset.set(5);
        s.scroll(ScrollDirection::PageUp, 0);
        assert_eq!(s.offset(), 5);
        s.scroll(ScrollDirection::PageDown, 0);
        assert_eq!(s.offset(), 5);
    }

    #[test]
    fn clamped_offset_normalizes_for_next_scroll() {
        let mut s = ScrollState::new();
        s.scroll_to_end();
        assert_eq!(s.offset(), usize::MAX);
        // clamped_offset normalizes the stored offset
        assert_eq!(s.clamped_offset(100, 10), 90);
        assert_eq!(s.offset(), 90);
        // Now scrolling up works from the normalized position
        s.scroll(ScrollDirection::Up, 10);
        assert_eq!(s.offset(), 89);
    }
}
