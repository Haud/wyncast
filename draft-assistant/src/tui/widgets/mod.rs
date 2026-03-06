// TUI widget modules for each dashboard panel.

use ratatui::style::{Color, Style};

pub mod budget;
pub mod nomination_banner;
pub mod position_filter_modal;
pub mod quit_confirm;
pub mod unsaved_changes_confirm;
pub mod status_bar;

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
