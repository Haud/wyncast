use iced::widget::operation::{self, AbsoluteOffset};
use iced::widget::scrollable::Viewport;
use iced::widget::Id as ScrollId;
use iced::{Element, Task};
use wyncast_app::protocol::{LlmStreamUpdate, ScrollDirection};

use crate::widgets::{StreamStatus, scrollable_markdown};
use crate::widgets::focus_ring;

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum PlanMessage {
    UserScrolled(f32),
    LlmUpdate { request_id: u64, update: LlmStreamUpdate },
    PlanStarted { request_id: u64 },
    NominationCleared,
    ScrollBy(ScrollDirection),
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct PlanPanel {
    text: String,
    status: StreamStatus,
    request_id: Option<u64>,
    scroll_id: ScrollId,
    auto_scroll: bool,
}

impl PlanPanel {
    pub fn new() -> Self {
        Self {
            text: String::new(),
            status: StreamStatus::Idle,
            request_id: None,
            scroll_id: ScrollId::unique(),
            auto_scroll: true,
        }
    }

    pub fn update(&mut self, msg: PlanMessage) -> Task<PlanMessage> {
        match msg {
            PlanMessage::UserScrolled(rel_y) => {
                self.auto_scroll = rel_y >= 0.99;
                Task::none()
            }
            PlanMessage::PlanStarted { request_id } => {
                self.text.clear();
                self.request_id = Some(request_id);
                self.status = StreamStatus::Streaming;
                self.auto_scroll = true;
                operation::snap_to_end(self.scroll_id.clone())
            }
            PlanMessage::LlmUpdate { request_id, update } => {
                if Some(request_id) != self.request_id {
                    return Task::none();
                }
                match update {
                    LlmStreamUpdate::Token(token) => {
                        self.text.push_str(&token);
                        self.status = StreamStatus::Streaming;
                        if self.auto_scroll {
                            operation::snap_to_end(self.scroll_id.clone())
                        } else {
                            Task::none()
                        }
                    }
                    LlmStreamUpdate::Complete(final_text) => {
                        self.text = final_text;
                        self.status = StreamStatus::Complete;
                        self.auto_scroll = false;
                        Task::none()
                    }
                    LlmStreamUpdate::Error(err) => {
                        self.status = StreamStatus::Error(err);
                        self.auto_scroll = false;
                        Task::none()
                    }
                }
            }
            PlanMessage::NominationCleared => {
                self.text.clear();
                self.status = StreamStatus::Idle;
                self.request_id = None;
                self.auto_scroll = true;
                Task::none()
            }
            PlanMessage::ScrollBy(dir) => {
                let (dx, dy) = scroll_amount(dir);
                if dy < 0.0 {
                    self.auto_scroll = false;
                }
                operation::scroll_by(self.scroll_id.clone(), AbsoluteOffset { x: dx, y: dy })
            }
        }
    }

    pub fn view(&self, focused: bool) -> Element<'_, PlanMessage> {
        let content = scrollable_markdown(
            &self.text,
            self.auto_scroll,
            self.scroll_id.clone(),
            &self.status,
            Some("Nomination Plan"),
            |vp: Viewport| PlanMessage::UserScrolled(vp.relative_offset().y),
        );
        focus_ring(content, focused)
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

    #[test]
    fn new_is_idle_with_empty_text() {
        let panel = PlanPanel::new();
        assert_eq!(panel.status, StreamStatus::Idle);
        assert!(panel.text.is_empty());
        assert!(panel.request_id.is_none());
        assert!(panel.auto_scroll);
    }

    #[test]
    fn plan_started_clears_and_sets_streaming() {
        let mut panel = PlanPanel::new();
        panel.text = "old".to_string();
        let _ = panel.update(PlanMessage::PlanStarted { request_id: 7 });
        assert!(panel.text.is_empty());
        assert_eq!(panel.request_id, Some(7));
        assert_eq!(panel.status, StreamStatus::Streaming);
        assert!(panel.auto_scroll);
    }

    #[test]
    fn llm_token_appends_when_id_matches() {
        let mut panel = PlanPanel::new();
        let _ = panel.update(PlanMessage::PlanStarted { request_id: 1 });
        let _ = panel.update(PlanMessage::LlmUpdate {
            request_id: 1,
            update: LlmStreamUpdate::Token("hello".to_string()),
        });
        assert_eq!(panel.text, "hello");
        assert_eq!(panel.status, StreamStatus::Streaming);
    }

    #[test]
    fn stale_request_id_discarded() {
        let mut panel = PlanPanel::new();
        let _ = panel.update(PlanMessage::PlanStarted { request_id: 1 });
        let _ = panel.update(PlanMessage::LlmUpdate {
            request_id: 99,
            update: LlmStreamUpdate::Token("stale".to_string()),
        });
        assert!(panel.text.is_empty());
    }

    #[test]
    fn llm_complete_sets_final_text_and_stops_auto_scroll() {
        let mut panel = PlanPanel::new();
        let _ = panel.update(PlanMessage::PlanStarted { request_id: 1 });
        let _ = panel.update(PlanMessage::LlmUpdate {
            request_id: 1,
            update: LlmStreamUpdate::Complete("final plan".to_string()),
        });
        assert_eq!(panel.text, "final plan");
        assert_eq!(panel.status, StreamStatus::Complete);
        assert!(!panel.auto_scroll);
    }

    #[test]
    fn llm_error_sets_error_status() {
        let mut panel = PlanPanel::new();
        let _ = panel.update(PlanMessage::PlanStarted { request_id: 1 });
        let _ = panel.update(PlanMessage::LlmUpdate {
            request_id: 1,
            update: LlmStreamUpdate::Error("timeout".to_string()),
        });
        assert!(matches!(panel.status, StreamStatus::Error(_)));
        assert!(!panel.auto_scroll);
    }

    #[test]
    fn nomination_cleared_resets() {
        let mut panel = PlanPanel::new();
        let _ = panel.update(PlanMessage::PlanStarted { request_id: 1 });
        let _ = panel.update(PlanMessage::LlmUpdate {
            request_id: 1,
            update: LlmStreamUpdate::Token("plan text".to_string()),
        });
        let _ = panel.update(PlanMessage::NominationCleared);
        assert!(panel.text.is_empty());
        assert_eq!(panel.status, StreamStatus::Idle);
        assert!(panel.request_id.is_none());
        assert!(panel.auto_scroll);
    }

    #[test]
    fn user_scroll_at_half_disables_auto_scroll() {
        let mut panel = PlanPanel::new();
        let _ = panel.update(PlanMessage::UserScrolled(0.5));
        assert!(!panel.auto_scroll);
    }

    #[test]
    fn user_scroll_at_bottom_re_enables_auto_scroll() {
        let mut panel = PlanPanel::new();
        panel.auto_scroll = false;
        let _ = panel.update(PlanMessage::UserScrolled(1.0));
        assert!(panel.auto_scroll);
    }

    #[test]
    fn scroll_by_up_disables_auto_scroll() {
        let mut panel = PlanPanel::new();
        let _ = panel.update(PlanMessage::ScrollBy(ScrollDirection::Up));
        assert!(!panel.auto_scroll);
    }

    #[test]
    fn scroll_by_down_preserves_auto_scroll() {
        let mut panel = PlanPanel::new();
        assert!(panel.auto_scroll);
        let _ = panel.update(PlanMessage::ScrollBy(ScrollDirection::Down));
        assert!(panel.auto_scroll);
    }
}
