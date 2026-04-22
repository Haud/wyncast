use iced::widget::operation::{self, AbsoluteOffset};
use iced::widget::scrollable::Viewport;
use iced::widget::Id as ScrollId;
use iced::{Element, Task};
use wyncast_app::protocol::{LlmStreamUpdate, ScrollDirection};

use crate::widgets::{StreamStatus, scrollable_markdown};
use super::super::DraftMessage;

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum AnalysisMessage {
    UserScrolled(f32),
    LlmUpdate { request_id: u64, update: LlmStreamUpdate },
    Nominated { analysis_request_id: Option<u64> },
    NominationCleared,
    ScrollBy(ScrollDirection),
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct AnalysisPanel {
     text: String,
     status: StreamStatus,
     request_id: Option<u64>,
     scroll_id: ScrollId,
     auto_scroll: bool,
}

impl AnalysisPanel {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            status: StreamStatus::Idle,
            request_id: None,
            scroll_id: ScrollId::unique(),
            auto_scroll: true,
        }
    }

    pub fn update(&mut self, msg: AnalysisMessage) -> Task<AnalysisMessage> {
        match msg {
            AnalysisMessage::UserScrolled(rel_y) => {
                self.handle_scroll(rel_y);
                Task::none()
            }
            AnalysisMessage::LlmUpdate { request_id, update } => {
                if self.apply_llm_update(request_id, &update) {
                    operation::snap_to_end(self.scroll_id.clone())
                } else {
                    Task::none()
                }
            }
            AnalysisMessage::Nominated { analysis_request_id } => {
                self.apply_nomination(analysis_request_id);
                operation::snap_to_end(self.scroll_id.clone())
            }
            AnalysisMessage::NominationCleared => {
                self.reset();
                Task::none()
            }
            AnalysisMessage::ScrollBy(dir) => {
                let (dx, dy) = scroll_amount(dir);
                if dy < 0.0 {
                    self.auto_scroll = false;
                }
                operation::scroll_by(self.scroll_id.clone(), AbsoluteOffset { x: dx, y: dy })
            }
        }
    }

    fn apply_llm_update(&mut self, request_id: u64, update: &LlmStreamUpdate) -> bool {
        if Some(request_id) != self.request_id {
            return false;
        }
        match update {
            LlmStreamUpdate::Token(token) => {
                self.text.push_str(token);
                self.status = StreamStatus::Streaming;
                self.auto_scroll
            }
            LlmStreamUpdate::Complete(final_text) => {
                self.text = final_text.clone();
                self.status = StreamStatus::Complete;
                self.auto_scroll = false;
                false
            }
            LlmStreamUpdate::Error(err) => {
                self.status = StreamStatus::Error(err.clone());
                self.auto_scroll = false;
                false
            }
        }
    }

    fn apply_nomination(&mut self, analysis_request_id: Option<u64>) {
        self.text.clear();
        self.request_id = analysis_request_id;
        self.status = if analysis_request_id.is_some() {
            StreamStatus::Streaming
        } else {
            StreamStatus::Idle
        };
        self.auto_scroll = true;
    }

    fn reset(&mut self) {
        self.text.clear();
        self.status = StreamStatus::Idle;
        self.request_id = None;
        self.auto_scroll = true;
    }

    fn handle_scroll(&mut self, rel_y: f32) {
        self.auto_scroll = rel_y >= 0.99;
    }

    pub fn view(&self) -> Element<'_, DraftMessage> {
        scrollable_markdown(
            &self.text,
            self.auto_scroll,
            self.scroll_id.clone(),
            &self.status,
            Some("LLM Analysis"),
            |viewport: Viewport| {
                DraftMessage::Analysis(AnalysisMessage::UserScrolled(
                    viewport.relative_offset().y,
                ))
            },
        )
    }
}

impl Default for AnalysisPanel {
    fn default() -> Self {
        Self::new()
    }
}

fn scroll_amount(dir: ScrollDirection) -> (f32, f32) {
    match dir {
        ScrollDirection::Up => (0.0, -40.0),
        ScrollDirection::Down => (0.0, 40.0),
        ScrollDirection::PageUp => (0.0, -300.0),
        ScrollDirection::PageDown => (0.0, 300.0),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn token(t: &str) -> LlmStreamUpdate {
        LlmStreamUpdate::Token(t.to_owned())
    }
    fn complete(t: &str) -> LlmStreamUpdate {
        LlmStreamUpdate::Complete(t.to_owned())
    }
    fn error(e: &str) -> LlmStreamUpdate {
        LlmStreamUpdate::Error(e.to_owned())
    }

    #[test]
    fn new_starts_idle_with_empty_text() {
        let panel = AnalysisPanel::new();
        assert_eq!(panel.status, StreamStatus::Idle);
        assert!(panel.text.is_empty());
        assert!(panel.auto_scroll);
        assert!(panel.request_id.is_none());
    }

    #[test]
    fn apply_token_appends_and_sets_streaming() {
        let mut panel = AnalysisPanel::new();
        panel.request_id = Some(1);
        let snap = panel.apply_llm_update(1, &token("hello "));
        assert_eq!(panel.text, "hello ");
        assert_eq!(panel.status, StreamStatus::Streaming);
        assert!(snap, "should request snap to bottom when auto_scroll is true");

        let snap2 = panel.apply_llm_update(1, &token("world"));
        assert_eq!(panel.text, "hello world");
        assert!(snap2);
    }

    #[test]
    fn apply_complete_sets_final_text_and_status() {
        let mut panel = AnalysisPanel::new();
        panel.request_id = Some(1);
        panel.apply_llm_update(1, &token("partial "));
        let snap = panel.apply_llm_update(1, &complete("final text"));
        assert_eq!(panel.text, "final text");
        assert_eq!(panel.status, StreamStatus::Complete);
        assert!(!snap, "Complete should not trigger auto-scroll");
        assert!(!panel.auto_scroll);
    }

    #[test]
    fn apply_error_sets_error_status() {
        let mut panel = AnalysisPanel::new();
        panel.request_id = Some(1);
        panel.apply_llm_update(1, &token("some text"));
        let snap = panel.apply_llm_update(1, &error("timeout"));
        assert!(matches!(panel.status, StreamStatus::Error(_)));
        if let StreamStatus::Error(msg) = &panel.status {
            assert_eq!(msg, "timeout");
        }
        assert!(!snap);
    }

    #[test]
    fn stale_request_id_is_discarded() {
        let mut panel = AnalysisPanel::new();
        panel.request_id = Some(2);
        let snap = panel.apply_llm_update(1, &token("stale"));
        assert!(panel.text.is_empty(), "stale tokens should be discarded");
        assert!(!snap);
    }

    #[test]
    fn nomination_clears_panel_and_sets_request_id() {
        let mut panel = AnalysisPanel::new();
        panel.request_id = Some(1);
        panel.apply_llm_update(1, &token("old text"));
        panel.apply_nomination(Some(2));
        assert!(panel.text.is_empty());
        assert_eq!(panel.request_id, Some(2));
        assert_eq!(panel.status, StreamStatus::Streaming);
        assert!(panel.auto_scroll);
    }

    #[test]
    fn nomination_without_request_id_sets_idle() {
        let mut panel = AnalysisPanel::new();
        panel.apply_nomination(None);
        assert_eq!(panel.status, StreamStatus::Idle);
        assert!(panel.request_id.is_none());
    }

    #[test]
    fn reset_restores_initial_state() {
        let mut panel = AnalysisPanel::new();
        panel.request_id = Some(1);
        panel.apply_llm_update(1, &token("text"));
        panel.reset();
        assert!(panel.text.is_empty());
        assert_eq!(panel.status, StreamStatus::Idle);
        assert!(panel.request_id.is_none());
        assert!(panel.auto_scroll);
    }

    #[test]
    fn handle_scroll_disables_auto_scroll_when_not_at_bottom() {
        let mut panel = AnalysisPanel::new();
        assert!(panel.auto_scroll);
        panel.handle_scroll(0.5);
        assert!(!panel.auto_scroll);
    }

    #[test]
    fn handle_scroll_re_enables_auto_scroll_at_bottom() {
        let mut panel = AnalysisPanel::new();
        panel.auto_scroll = false;
        panel.handle_scroll(1.0);
        assert!(panel.auto_scroll);
    }

    #[test]
    fn handle_scroll_threshold_at_0_99() {
        let mut panel = AnalysisPanel::new();
        panel.handle_scroll(0.989);
        assert!(!panel.auto_scroll);
        panel.handle_scroll(0.99);
        assert!(panel.auto_scroll);
    }

    #[test]
    fn auto_scroll_disabled_token_returns_false() {
        let mut panel = AnalysisPanel::new();
        panel.request_id = Some(1);
        panel.auto_scroll = false;
        let snap = panel.apply_llm_update(1, &token("hi"));
        assert!(!snap, "should not snap when auto_scroll is disabled");
    }

    // --- Tests exercising the update() entry point ---

    #[test]
    fn update_user_scrolled_sets_auto_scroll() {
        let mut panel = AnalysisPanel::new();
        assert!(panel.auto_scroll);
        let _ = panel.update(AnalysisMessage::UserScrolled(0.5));
        assert!(!panel.auto_scroll);
        let _ = panel.update(AnalysisMessage::UserScrolled(0.99));
        assert!(panel.auto_scroll);
    }

    #[test]
    fn update_llm_token_appends_text() {
        let mut panel = AnalysisPanel::new();
        panel.request_id = Some(1);
        let _ = panel.update(AnalysisMessage::LlmUpdate {
            request_id: 1,
            update: token("hello"),
        });
        assert_eq!(panel.text, "hello");
        assert_eq!(panel.status, StreamStatus::Streaming);
    }

    #[test]
    fn update_nominated_resets_for_new_stream() {
        let mut panel = AnalysisPanel::new();
        panel.request_id = Some(1);
        panel.apply_llm_update(1, &token("old"));
        let _ = panel.update(AnalysisMessage::Nominated { analysis_request_id: Some(2) });
        assert!(panel.text.is_empty());
        assert_eq!(panel.request_id, Some(2));
        assert_eq!(panel.status, StreamStatus::Streaming);
    }

    #[test]
    fn update_nomination_cleared_resets_to_idle() {
        let mut panel = AnalysisPanel::new();
        panel.request_id = Some(1);
        panel.apply_llm_update(1, &token("text"));
        let _ = panel.update(AnalysisMessage::NominationCleared);
        assert!(panel.text.is_empty());
        assert_eq!(panel.status, StreamStatus::Idle);
        assert!(panel.request_id.is_none());
    }

    #[test]
    fn update_scroll_by_up_disables_auto_scroll() {
        let mut panel = AnalysisPanel::new();
        assert!(panel.auto_scroll);
        let _ = panel.update(AnalysisMessage::ScrollBy(ScrollDirection::Up));
        assert!(!panel.auto_scroll);
    }

    #[test]
    fn update_scroll_by_down_preserves_auto_scroll() {
        let mut panel = AnalysisPanel::new();
        assert!(panel.auto_scroll);
        let _ = panel.update(AnalysisMessage::ScrollBy(ScrollDirection::Down));
        assert!(panel.auto_scroll);
    }
}
