// TUI widget modules for each dashboard panel.

use ratatui::style::{Color, Style};

pub mod available;
pub mod budget;
pub mod draft_log;
pub mod llm_analysis;
pub mod nomination_banner;
pub mod nomination_plan;
pub mod quit_confirm;
pub mod roster;
pub mod scarcity;
pub mod status_bar;
pub mod teams;

/// Return a cyan border style when focused, otherwise the provided base style.
///
/// This is the single source of truth for focus-highlight borders across all
/// widgets, ensuring consistent visual feedback when a panel has keyboard focus.
pub fn focused_border_style(focused: bool, base_style: Style) -> Style {
    if focused {
        Style::default().fg(Color::Cyan)
    } else {
        base_style
    }
}
