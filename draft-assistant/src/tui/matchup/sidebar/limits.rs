// Limits panel: GS limit, acquisitions, days remaining, and games today.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::tui::action::Action;
use crate::tui::scroll::{ScrollDirection, ScrollState};
use crate::tui::widgets::focused_border_style;

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
        frame: &mut Frame,
        area: Rect,
        data: &LimitsData,
        focused: bool,
    ) {
        let border = focused_border_style(focused, Style::default());
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Limits & Resources ")
            .border_style(border);

        let inner_width = area.width.saturating_sub(2) as usize;
        let viewport_height = area.height.saturating_sub(2) as usize;

        let mut lines: Vec<Line<'static>> = Vec::new();

        // Games Started
        lines.push(Line::from(Span::styled(
            " Games Started (GS)",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(build_progress_line(
            data.gs_used as usize,
            data.gs_limit as usize,
            inner_width,
        ));
        lines.push(Line::from(""));

        // Acquisitions
        lines.push(Line::from(Span::styled(
            " Acquisitions",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(build_progress_line(
            data.acq_used as usize,
            data.acq_limit as usize,
            inner_width,
        ));
        lines.push(Line::from(""));

        // Days remaining
        lines.push(Line::from(vec![
            Span::styled(" Days Remaining: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{}", data.days_remaining),
                Style::default().fg(Color::White),
            ),
        ]));

        // Games today
        lines.push(Line::from(vec![
            Span::styled(" Games Today: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{}", data.games_today),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!(" of {} roster spots", data.total_active),
                Style::default().fg(Color::DarkGray),
            ),
        ]));

        let content_height = lines.len();
        let scroll_offset = self.scroll.clamped_offset(content_height, viewport_height);

        let paragraph = Paragraph::new(lines)
            .block(block)
            .scroll((scroll_offset as u16, 0));
        frame.render_widget(paragraph, area);
    }
}

impl Default for LimitsPanel {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Progress bar helpers
// ---------------------------------------------------------------------------

/// Determine progress bar color based on usage percentage.
pub(crate) fn progress_color(used: usize, limit: usize) -> Color {
    if limit == 0 {
        return Color::Gray;
    }
    let pct = (used as f64 / limit as f64) * 100.0;
    if pct > 85.0 {
        Color::Red
    } else if pct >= 60.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}

/// Build a progress bar line: ` ████████░░░░  used/limit`
fn build_progress_line(used: usize, limit: usize, available_width: usize) -> Line<'static> {
    let label = format!(" {}/{}", used, limit);
    let label_width = label.len() + 1; // +1 for leading space before bar
    let bar_width = available_width.saturating_sub(label_width).saturating_sub(1);

    let filled = if limit == 0 {
        0
    } else {
        ((bar_width as f64 * used as f64 / limit as f64).round() as usize).min(bar_width)
    };
    let empty = bar_width.saturating_sub(filled);

    let color = progress_color(used, limit);

    Line::from(vec![
        Span::raw(" "),
        Span::styled(
            "\u{2588}".repeat(filled),
            Style::default().fg(color),
        ),
        Span::styled(
            "\u{2591}".repeat(empty),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(label, Style::default().fg(Color::White)),
    ])
}
