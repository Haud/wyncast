// Status bar widget: connection status, draft progress, tab indicator.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::protocol::{ConnectionStatus, TabId};
use crate::tui::ViewState;

/// Render the status bar into the given area.
///
/// Layout: [connection indicator] [pick counter] [tab bar]
pub fn render(frame: &mut Frame, area: Rect, state: &ViewState) {
    let mut spans = Vec::new();

    // Connection indicator
    let (dot, dot_color) = connection_indicator(state.connection_status);
    spans.push(Span::styled(
        format!(" {} ", dot),
        Style::default().fg(dot_color),
    ));

    // Pick counter
    spans.push(Span::styled(
        format!("Pick {}/{}", state.pick_number, state.total_picks),
        Style::default().fg(Color::White),
    ));

    // Separator
    spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));

    // Tab bar
    let tabs = tab_spans(state.active_tab);
    spans.extend(tabs);

    let paragraph = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(Color::DarkGray));
    frame.render_widget(paragraph, area);
}

/// Return the connection dot character and its color.
pub fn connection_indicator(status: ConnectionStatus) -> (&'static str, Color) {
    match status {
        ConnectionStatus::Connected => ("●", Color::Green),
        ConnectionStatus::Disconnected => ("●", Color::Red),
    }
}

/// Build tab indicator spans: "[1] [2] [3] [4] [5]" with active highlighted.
pub fn tab_spans(active: TabId) -> Vec<Span<'static>> {
    let tabs = [
        (TabId::Analysis, "1"),
        (TabId::NomPlan, "2"),
        (TabId::Available, "3"),
        (TabId::DraftLog, "4"),
        (TabId::Teams, "5"),
    ];

    let mut spans = Vec::new();
    for (tab_id, label) in tabs {
        let style = if tab_id == active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(format!("[{}]", label), style));
        spans.push(Span::raw(" "));
    }
    spans
}

/// Return the label for a tab.
pub fn tab_label(tab: TabId) -> &'static str {
    match tab {
        TabId::Analysis => "Analysis",
        TabId::NomPlan => "Nom Plan",
        TabId::Available => "Available",
        TabId::DraftLog => "Draft Log",
        TabId::Teams => "Teams",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_indicator_connected() {
        let (dot, color) = connection_indicator(ConnectionStatus::Connected);
        assert_eq!(dot, "●");
        assert_eq!(color, Color::Green);
    }

    #[test]
    fn connection_indicator_disconnected() {
        let (dot, color) = connection_indicator(ConnectionStatus::Disconnected);
        assert_eq!(dot, "●");
        assert_eq!(color, Color::Red);
    }

    #[test]
    fn tab_spans_highlight_active() {
        let spans = tab_spans(TabId::Available);
        // The 3rd tab (index 4 in spans: [1] " " [2] " " [3] ...)
        // [3] should be highlighted
        let tab3 = &spans[4]; // 0=[1], 1=" ", 2=[2], 3=" ", 4=[3]
        assert!(tab3.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn tab_label_values() {
        assert_eq!(tab_label(TabId::Analysis), "Analysis");
        assert_eq!(tab_label(TabId::NomPlan), "Nom Plan");
        assert_eq!(tab_label(TabId::Available), "Available");
        assert_eq!(tab_label(TabId::DraftLog), "Draft Log");
        assert_eq!(tab_label(TabId::Teams), "Teams");
    }

    #[test]
    fn render_does_not_panic_with_defaults() {
        let backend = ratatui::backend::TestBackend::new(80, 1);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = ViewState::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }
}
