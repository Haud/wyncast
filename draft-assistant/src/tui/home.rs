// Home screen: shown in Draft mode when the ESPN extension is not connected.
//
// Displays the WYNCAST ASCII art logo with a "waiting for connection" indicator
// and minimal keybind hints (quit, settings).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crossterm::event::KeyCode;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::app::App;
use crate::tui::subscription::keybinding::{
    exact, KeyBindingRecipe, KeybindHint, KeybindManager,
};
use crate::tui::subscription::{Subscription, SubscriptionId};

// ---------------------------------------------------------------------------
// HomeMessage
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum HomeMessage {
    OpenSettings,
    Quit,
}

// ---------------------------------------------------------------------------
// Subscription
// ---------------------------------------------------------------------------

pub fn subscription(kb: &mut KeybindManager) -> Subscription<HomeMessage> {
    let mut h = DefaultHasher::new();
    "home-screen".hash(&mut h);
    let sub_id = SubscriptionId::from_u64(h.finish());

    kb.subscribe(
        KeyBindingRecipe::new(sub_id)
            .bind(
                exact(KeyCode::Char('q')),
                |_| HomeMessage::Quit,
                KeybindHint::new("q", "Quit"),
            )
            .bind(
                exact(KeyCode::Char(',')),
                |_| HomeMessage::OpenSettings,
                KeybindHint::new(",", "Settings"),
            ),
    )
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

const ASCII_ART: &[&str] = &[
    "█   █ █ █ █   █ █████  ███  █████ █████",
    "█   █ █ █ ██  █ █     █   █ █       █  ",
    "█ █ █  █  █ █ █ █     █████  ███    █  ",
    "█ █ █  █  █  ██ █     █   █     █   █  ",
    " █ █   █  █   █ █████ █   █ █████   █  ",
];

const ART_HEIGHT: u16 = 5;

pub fn render(frame: &mut Frame, app: &App) {
    let frame_area = frame.area();

    // Split into content area + 1-line help bar at bottom.
    let [content_area, help_area] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(frame_area);

    // -- Centered content --
    // Total content height: art (5) + blank (1) + subtitle (1) + blank (1) + status (1) = 9
    let content_height: u16 = ART_HEIGHT + 4;

    // Vertical centering.
    let y_offset = if content_area.height > content_height {
        (content_area.height - content_height) / 2
    } else {
        0
    };

    let centered_area = Rect {
        x: content_area.x,
        y: content_area.y + y_offset,
        width: content_area.width,
        height: content_height.min(content_area.height.saturating_sub(y_offset)),
    };

    // Build lines.
    let art_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let subtitle_style = Style::default().fg(Color::DarkGray);

    let mut lines: Vec<Line> = Vec::new();

    // ASCII art lines.
    for &art_line in ASCII_ART {
        lines.push(Line::from(Span::styled(art_line, art_style)));
    }

    // Blank line.
    lines.push(Line::from(""));

    // Subtitle.
    lines.push(Line::from(Span::styled(
        "Navigate to a supported ESPN page to begin.",
        subtitle_style,
    )));

    // Blank line.
    lines.push(Line::from(""));

    // Blinking connection status.
    let dot = if app.tick_count % 2 == 0 { "● " } else { "  " };
    let status_line = Line::from(vec![
        Span::styled(dot, Style::default().fg(Color::Red)),
        Span::styled("Waiting for connection...", subtitle_style),
    ]);
    lines.push(status_line);

    let paragraph = Paragraph::new(lines).alignment(Alignment::Center);
    frame.render_widget(paragraph, centered_area);

    // -- Help bar --
    super::render_keybind_hints(frame, help_area, &app.active_keybinds);
}
