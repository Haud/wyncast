use crate::protocol::LlmStatus;
use crate::tui::action::Action;
use crate::tui::scroll::{ScrollDirection, ScrollState};

/// Reusable state for an LLM streaming text panel.
/// Used by both AnalysisPanel and PlanPanel.
#[derive(Debug, Clone)]
pub struct LlmStreamState {
    pub text: String,
    pub status: LlmStatus,
    scroll: ScrollState,
}

/// Messages that can be sent to an `LlmStreamState` component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmStreamMessage {
    TokenReceived(String),
    Complete(String),
    Error(String),
    Clear,
    Scroll(ScrollDirection),
}

impl LlmStreamState {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            status: LlmStatus::Idle,
            scroll: ScrollState::new(),
        }
    }

    /// Get the scroll offset clamped for the given viewport height.
    /// Content height is derived from `self.text`.
    pub fn scroll_offset_clamped(&self, viewport_height: usize) -> usize {
        let content_height = self.text.lines().count();
        self.scroll.clamped_offset(content_height, viewport_height)
    }

    /// Raw scroll offset (may exceed valid range).
    pub fn scroll_offset(&self) -> usize {
        self.scroll.offset()
    }

    pub fn update(&mut self, msg: LlmStreamMessage) -> Option<Action> {
        match msg {
            LlmStreamMessage::TokenReceived(token) => {
                self.text.push_str(&token);
                self.status = LlmStatus::Streaming;
                self.scroll.scroll_to_end();
                None
            }
            LlmStreamMessage::Complete(final_text) => {
                self.text = final_text;
                self.status = LlmStatus::Complete;
                None
            }
            LlmStreamMessage::Error(err) => {
                self.text.clear();
                self.text.push_str(&format!("[Error: {}]", err));
                self.status = LlmStatus::Error;
                None
            }
            LlmStreamMessage::Clear => {
                self.text.clear();
                self.status = LlmStatus::Idle;
                self.scroll = ScrollState::new();
                None
            }
            LlmStreamMessage::Scroll(dir) => {
                self.scroll.scroll(dir, 0);
                None
            }
        }
    }

    // NOTE: view() is intentionally NOT included here.
    // Each usage (Analysis, Plan) has different chrome (title, border, status indicator).
    // The parent component handles rendering using self.text, self.status, self.scroll.
}

impl Default for LlmStreamState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_empty_idle_zeroed() {
        let s = LlmStreamState::new();
        assert_eq!(s.text, "");
        assert_eq!(s.status, LlmStatus::Idle);
        assert_eq!(s.scroll_offset(), 0);
    }

    #[test]
    fn default_starts_empty_idle_zeroed() {
        let s = LlmStreamState::default();
        assert_eq!(s.text, "");
        assert_eq!(s.status, LlmStatus::Idle);
        assert_eq!(s.scroll_offset(), 0);
    }

    #[test]
    fn token_received_appends_text_and_sets_streaming() {
        let mut s = LlmStreamState::new();
        let result = s.update(LlmStreamMessage::TokenReceived("hello".into()));
        assert_eq!(s.text, "hello");
        assert_eq!(s.status, LlmStatus::Streaming);
        assert_eq!(result, None);
    }

    #[test]
    fn token_received_scrolls_to_end() {
        let mut s = LlmStreamState::new();
        s.update(LlmStreamMessage::TokenReceived("token".into()));
        // scroll_to_end sets offset to usize::MAX; clamped_offset resolves it
        assert_eq!(s.scroll_offset(), usize::MAX);
        // With 1 line of content and viewport of 10, clamped is 0
        assert_eq!(s.scroll_offset_clamped(10), 0);
    }

    #[test]
    fn multiple_tokens_accumulate() {
        let mut s = LlmStreamState::new();
        s.update(LlmStreamMessage::TokenReceived("hello ".into()));
        s.update(LlmStreamMessage::TokenReceived("world".into()));
        assert_eq!(s.text, "hello world");
        assert_eq!(s.status, LlmStatus::Streaming);
    }

    #[test]
    fn complete_replaces_text_entirely() {
        let mut s = LlmStreamState::new();
        s.update(LlmStreamMessage::TokenReceived("partial".into()));
        let result = s.update(LlmStreamMessage::Complete("final text".into()));
        assert_eq!(s.text, "final text");
        assert_eq!(s.status, LlmStatus::Complete);
        assert_eq!(result, None);
    }

    #[test]
    fn error_clears_and_sets_error_message() {
        let mut s = LlmStreamState::new();
        s.update(LlmStreamMessage::TokenReceived("some text".into()));
        let result = s.update(LlmStreamMessage::Error("timeout".into()));
        assert_eq!(s.text, "[Error: timeout]");
        assert_eq!(s.status, LlmStatus::Error);
        assert_eq!(result, None);
    }

    #[test]
    fn clear_resets_everything() {
        let mut s = LlmStreamState::new();
        s.update(LlmStreamMessage::TokenReceived("data".into()));
        s.update(LlmStreamMessage::Scroll(ScrollDirection::Down));

        let result = s.update(LlmStreamMessage::Clear);
        assert_eq!(s.text, "");
        assert_eq!(s.status, LlmStatus::Idle);
        assert_eq!(s.scroll_offset(), 0);
        assert_eq!(result, None);
    }

    #[test]
    fn scroll_delegates_to_scroll_state() {
        let mut s = LlmStreamState::new();
        let result = s.update(LlmStreamMessage::Scroll(ScrollDirection::Down));
        assert_eq!(s.scroll_offset(), 1);
        assert_eq!(result, None);

        s.update(LlmStreamMessage::Scroll(ScrollDirection::Down));
        assert_eq!(s.scroll_offset(), 2);
    }

    #[test]
    fn token_after_clear_starts_fresh() {
        let mut s = LlmStreamState::new();
        s.update(LlmStreamMessage::TokenReceived("old".into()));
        s.update(LlmStreamMessage::Clear);
        s.update(LlmStreamMessage::TokenReceived("new".into()));
        assert_eq!(s.text, "new");
        assert_eq!(s.status, LlmStatus::Streaming);
    }

    #[test]
    fn all_updates_return_none() {
        let mut s = LlmStreamState::new();
        assert_eq!(s.update(LlmStreamMessage::TokenReceived("t".into())), None);
        assert_eq!(s.update(LlmStreamMessage::Complete("c".into())), None);
        assert_eq!(s.update(LlmStreamMessage::Error("e".into())), None);
        assert_eq!(s.update(LlmStreamMessage::Clear), None);
        assert_eq!(
            s.update(LlmStreamMessage::Scroll(ScrollDirection::Up)),
            None
        );
    }
}
