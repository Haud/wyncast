use iced::widget::Id as WidgetId;
use iced::widget::scrollable::Viewport;
use iced::{Element, Length, Task};
use twui::{BoxStyle, frame};
use wyncast_app::protocol::ScrollDirection;

use crate::widgets::{StreamStatus, scrollable_markdown};
use crate::widgets::data_table::ROW_HEIGHT;

#[derive(Debug, Clone)]
pub enum AnalyticsMessage {
    #[allow(dead_code)]
    Scrolled(Viewport),
    ScrollBy(ScrollDirection),
}

pub struct AnalyticsPanel {
    scroll_id: WidgetId,
}

impl AnalyticsPanel {
    pub fn new() -> Self {
        Self { scroll_id: WidgetId::unique() }
    }

    pub fn update(&mut self, msg: AnalyticsMessage) -> Task<AnalyticsMessage> {
        match msg {
            AnalyticsMessage::Scrolled(_) => Task::none(),
            AnalyticsMessage::ScrollBy(dir) => {
                use iced::widget::operation::{self, AbsoluteOffset};
                let dy = match dir {
                    ScrollDirection::Up => -(ROW_HEIGHT * 3.0),
                    ScrollDirection::Down => ROW_HEIGHT * 3.0,
                    ScrollDirection::PageUp => -(ROW_HEIGHT * 10.0),
                    ScrollDirection::PageDown => ROW_HEIGHT * 10.0,
                };
                operation::scroll_by(self.scroll_id.clone(), AbsoluteOffset { x: 0.0, y: dy })
            }
        }
    }

    pub fn view(&self) -> Element<'_, AnalyticsMessage> {
        let content = scrollable_markdown(
            "# Matchup Analytics\n\nDetailed analytics are not yet available for this matchup.\n\nCheck back after the extension sends matchup data.",
            false,
            self.scroll_id.clone(),
            &StreamStatus::Idle,
            Some("Analytics"),
            AnalyticsMessage::Scrolled,
        );

        frame(
            content,
            BoxStyle { width: Length::Fill, height: Length::Fill, ..Default::default() },
        )
        .into()
    }
}

impl Default for AnalyticsPanel {
    fn default() -> Self {
        Self::new()
    }
}
