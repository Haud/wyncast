use iced::widget::scrollable::{self, Viewport};
use iced::widget::Id as ScrollId;
use iced::{Element, Length, Padding};
use twui::{
    Colors, Opacity, SpinnerSize, SpinnerStyle, TextColor, TextSize, TextStyle,
    BoxStyle, StackGap, StackStyle,
    frame, h_stack, markdown_renderer, spinner, text, v_stack,
};

/// Streaming status for an LLM-driven panel.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamStatus {
    Idle,
    Streaming,
    Complete,
    Error(String),
}

/// Thin wrapper combining `markdown_renderer` + `iced::widget::scrollable`
/// with a streaming header (title + spinner when streaming, error banner when Error).
///
/// Auto-scroll to bottom must be driven externally via Task (see `AnalysisPanel`).
/// The `auto_scroll` flag is accepted for API symmetry and to silence dead_code
/// warnings on the caller side; it does not affect rendering.
///
/// Flagged for later upstreaming to twui.
pub fn scrollable_markdown<'a, Message: Clone + 'a>(
    content: &str,
    _auto_scroll: bool,
    scroll_id: ScrollId,
    status: &StreamStatus,
    header: Option<&str>,
    on_scroll: impl Fn(Viewport) -> Message + 'a,
) -> Element<'a, Message> {
    let header_elem = build_header(header, status);
    let body = build_body(content, scroll_id, status, on_scroll);

    let mut children: Vec<Element<Message>> = vec![header_elem];

    if let StreamStatus::Error(msg) = status {
        children.push(error_banner(msg));
    }

    children.push(body);

    v_stack(
        children,
        StackStyle {
            gap: StackGap::None,
            width: Length::Fill,
            height: Length::Fill,
            ..Default::default()
        },
    )
    .into()
}

fn build_header<'a, Message: Clone + 'a>(
    title: Option<&str>,
    status: &StreamStatus,
) -> Element<'a, Message> {
    let title_str = title.unwrap_or("Analysis");
    let title_elem: Element<Message> = text(
        title_str,
        TextStyle {
            size: TextSize::Sm,
            color: TextColor::Default,
            ..Default::default()
        },
    )
    .into();

    let mut row_children: Vec<Element<Message>> = vec![title_elem];

    if *status == StreamStatus::Streaming {
        let spin: Element<Message> = spinner(SpinnerStyle::new().size(SpinnerSize::Sm)).into();
        row_children.push(spin);
    }

    h_stack(
        row_children,
        StackStyle {
            gap: StackGap::Xs,
            width: Length::Fill,
            padding: Padding::new(6.0),
            ..Default::default()
        },
    )
    .into()
}

fn error_banner<'a, Message: Clone + 'a>(msg: &str) -> Element<'a, Message> {
    let err_color = Colors::Destructive.rgba(Opacity::O20);
    let label: Element<Message> = text(
        msg,
        TextStyle {
            size: TextSize::Xs,
            color: TextColor::Error,
            ..Default::default()
        },
    )
    .into();

    iced::widget::container(label)
        .style(move |_| iced::widget::container::Style {
            background: Some(iced::Background::Color(err_color)),
            ..Default::default()
        })
        .width(Length::Fill)
        .padding(Padding::new(6.0))
        .into()
}

fn build_body<'a, Message: Clone + 'a>(
    content: &str,
    scroll_id: ScrollId,
    status: &StreamStatus,
    on_scroll: impl Fn(Viewport) -> Message + 'a,
) -> Element<'a, Message> {
    let display = if content.is_empty() {
        placeholder_text(status)
    } else {
        content.to_owned()
    };

    let md: Element<Message> = frame(
        markdown_renderer::<Message>(&display),
        BoxStyle {
            width: Length::Fill,
            height: Length::Shrink,
            padding: Padding::new(8.0),
            ..Default::default()
        },
    )
    .into();

    scrollable::Scrollable::new(md)
        .id(scroll_id)
        .on_scroll(on_scroll)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn placeholder_text(status: &StreamStatus) -> String {
    match status {
        StreamStatus::Idle => "Waiting for nomination…".to_owned(),
        StreamStatus::Streaming => "Streaming…".to_owned(),
        StreamStatus::Complete => String::new(),
        StreamStatus::Error(_) => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_idle() {
        assert_eq!(placeholder_text(&StreamStatus::Idle), "Waiting for nomination…");
    }

    #[test]
    fn placeholder_streaming() {
        assert_eq!(placeholder_text(&StreamStatus::Streaming), "Streaming…");
    }

    #[test]
    fn placeholder_complete_empty() {
        assert!(placeholder_text(&StreamStatus::Complete).is_empty());
    }

    #[test]
    fn stream_status_eq() {
        assert_eq!(StreamStatus::Idle, StreamStatus::Idle);
        assert_ne!(StreamStatus::Idle, StreamStatus::Streaming);
        assert_eq!(StreamStatus::Error("x".into()), StreamStatus::Error("x".into()));
        assert_ne!(StreamStatus::Error("a".into()), StreamStatus::Error("b".into()));
    }
}
