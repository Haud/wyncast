// Strategy setup screen: budget split, category weights, and LLM-assisted configuration.
//
// This is Step 2 of the onboarding wizard. The user can either describe their
// strategy in natural language (AI mode) or manually edit the budget split and
// category weights (Manual mode). AI-generated configs populate the manual
// fields for review before saving.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::TextInput;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Category display names and field labels in display order.
///
/// The order is: hitting categories first (R, HR, RBI, BB, SB, AVG),
/// then pitching categories (K, W, SV, HD, ERA, WHIP). This matches
/// the league's configured category order.
pub const CATEGORIES: &[&str] = &[
    "R", "HR", "RBI", "BB", "SB", "AVG", "K", "W", "SV", "HD", "ERA", "WHIP",
];

/// Number of columns in the category weight grid.
const WEIGHT_COLS: usize = 3;

// ---------------------------------------------------------------------------
// StrategySetupMode
// ---------------------------------------------------------------------------

/// Whether the user is configuring via AI or manual input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategySetupMode {
    Ai,
    Manual,
}

// ---------------------------------------------------------------------------
// StrategySection
// ---------------------------------------------------------------------------

/// Which section of the strategy screen currently has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategySection {
    ModeToggle,
    AiInput,
    GenerateButton,
    BudgetField,
    CategoryWeights,
}

impl StrategySection {
    /// Ordered list of sections for Tab cycling in AI mode.
    const AI_CYCLE: &[StrategySection] = &[
        StrategySection::ModeToggle,
        StrategySection::AiInput,
        StrategySection::GenerateButton,
        StrategySection::BudgetField,
        StrategySection::CategoryWeights,
    ];

    /// Ordered list of sections for Tab cycling in Manual mode.
    const MANUAL_CYCLE: &[StrategySection] = &[
        StrategySection::ModeToggle,
        StrategySection::BudgetField,
        StrategySection::CategoryWeights,
    ];

    /// Advance to the next section (wraps around).
    pub fn next(self, mode: StrategySetupMode) -> StrategySection {
        let cycle = match mode {
            StrategySetupMode::Ai => Self::AI_CYCLE,
            StrategySetupMode::Manual => Self::MANUAL_CYCLE,
        };
        let idx = cycle.iter().position(|&s| s == self).unwrap_or(0);
        cycle[(idx + 1) % cycle.len()]
    }

    /// Go to the previous section (wraps around).
    pub fn prev(self, mode: StrategySetupMode) -> StrategySection {
        let cycle = match mode {
            StrategySetupMode::Ai => Self::AI_CYCLE,
            StrategySetupMode::Manual => Self::MANUAL_CYCLE,
        };
        let idx = cycle.iter().position(|&s| s == self).unwrap_or(0);
        if idx == 0 {
            cycle[cycle.len() - 1]
        } else {
            cycle[idx - 1]
        }
    }
}

// ---------------------------------------------------------------------------
// CategoryWeights
// ---------------------------------------------------------------------------

/// Category weight multipliers for all 12 league categories.
///
/// Provides indexed access via `get(idx)` and `set(idx, val)` using the
/// `CATEGORIES` const array ordering.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryWeights {
    pub r: f32,
    pub hr: f32,
    pub rbi: f32,
    pub bb: f32,
    pub sb: f32,
    pub avg: f32,
    pub k: f32,
    pub w: f32,
    pub sv: f32,
    pub hd: f32,
    pub era: f32,
    pub whip: f32,
}

impl Default for CategoryWeights {
    fn default() -> Self {
        CategoryWeights {
            r: 1.0,
            hr: 1.0,
            rbi: 1.0,
            bb: 1.0,
            sb: 1.0,
            avg: 1.0,
            k: 1.0,
            w: 1.0,
            sv: 0.7,
            hd: 1.0,
            era: 1.0,
            whip: 1.0,
        }
    }
}

impl CategoryWeights {
    /// Get the weight value at the given index (follows CATEGORIES order).
    pub fn get(&self, idx: usize) -> f32 {
        match idx {
            0 => self.r,
            1 => self.hr,
            2 => self.rbi,
            3 => self.bb,
            4 => self.sb,
            5 => self.avg,
            6 => self.k,
            7 => self.w,
            8 => self.sv,
            9 => self.hd,
            10 => self.era,
            11 => self.whip,
            _ => 1.0,
        }
    }

    /// Set the weight value at the given index (follows CATEGORIES order).
    pub fn set(&mut self, idx: usize, val: f32) {
        match idx {
            0 => self.r = val,
            1 => self.hr = val,
            2 => self.rbi = val,
            3 => self.bb = val,
            4 => self.sb = val,
            5 => self.avg = val,
            6 => self.k = val,
            7 => self.w = val,
            8 => self.sv = val,
            9 => self.hd = val,
            10 => self.era = val,
            11 => self.whip = val,
            _ => {}
        }
    }

    /// Convert to the config-compatible `CategoryWeights` (f64, uppercase field names).
    pub fn to_config_weights(&self) -> crate::config::CategoryWeights {
        crate::config::CategoryWeights {
            R: self.r as f64,
            HR: self.hr as f64,
            RBI: self.rbi as f64,
            BB: self.bb as f64,
            SB: self.sb as f64,
            AVG: self.avg as f64,
            K: self.k as f64,
            W: self.w as f64,
            SV: self.sv as f64,
            HD: self.hd as f64,
            ERA: self.era as f64,
            WHIP: self.whip as f64,
        }
    }

    /// Create from the config-compatible `CategoryWeights`.
    pub fn from_config_weights(w: &crate::config::CategoryWeights) -> Self {
        CategoryWeights {
            r: w.R as f32,
            hr: w.HR as f32,
            rbi: w.RBI as f32,
            bb: w.BB as f32,
            sb: w.SB as f32,
            avg: w.AVG as f32,
            k: w.K as f32,
            w: w.W as f32,
            sv: w.SV as f32,
            hd: w.HD as f32,
            era: w.ERA as f32,
            whip: w.WHIP as f32,
        }
    }
}

// ---------------------------------------------------------------------------
// StrategySetupState
// ---------------------------------------------------------------------------

/// UI state for the strategy setup screen.
///
/// Lives inside `ViewState` so the TUI can render it without any global state.
#[derive(Debug, Clone)]
pub struct StrategySetupState {
    /// AI or Manual mode.
    pub mode: StrategySetupMode,
    /// Which section currently has keyboard focus.
    pub active_section: StrategySection,
    /// Text area content for AI strategy description (with cursor tracking).
    pub ai_input: TextInput,
    /// Whether the AI text input is in edit mode.
    pub ai_input_editing: bool,
    /// Whether the LLM is currently generating.
    pub generating: bool,
    /// Streamed LLM output text.
    pub generation_output: String,
    /// Error message from LLM generation, if any.
    pub generation_error: Option<String>,
    /// Hitting budget percentage (0-100).
    pub hitting_budget_pct: u8,
    /// Category weight values.
    pub category_weights: CategoryWeights,
    /// Which field is being edited (None = not editing, Some("budget") or
    /// Some(category name)).
    pub editing_field: Option<String>,
    /// Current text being typed in an editable numeric field (with cursor tracking).
    pub field_input: TextInput,
    /// Which category weight is highlighted (0-11).
    pub selected_weight_idx: usize,
}

impl Default for StrategySetupState {
    fn default() -> Self {
        StrategySetupState {
            mode: StrategySetupMode::Ai,
            active_section: StrategySection::ModeToggle,
            ai_input: TextInput::new(),
            ai_input_editing: false,
            generating: false,
            generation_output: String::new(),
            generation_error: None,
            hitting_budget_pct: 65,
            category_weights: CategoryWeights::default(),
            editing_field: None,
            field_input: TextInput::new(),
            selected_weight_idx: 0,
        }
    }
}

impl StrategySetupState {
    /// Toggle between AI and Manual mode.
    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            StrategySetupMode::Ai => StrategySetupMode::Manual,
            StrategySetupMode::Manual => StrategySetupMode::Ai,
        };
        // Adjust active section if it doesn't exist in the new mode
        match self.mode {
            StrategySetupMode::Manual => {
                if self.active_section == StrategySection::AiInput
                    || self.active_section == StrategySection::GenerateButton
                {
                    self.active_section = StrategySection::BudgetField;
                }
            }
            StrategySetupMode::Ai => {
                // All sections are valid in AI mode
            }
        }
    }

    /// Move the selected weight index up (wraps).
    pub fn weight_up(&mut self) {
        if self.selected_weight_idx >= WEIGHT_COLS {
            self.selected_weight_idx -= WEIGHT_COLS;
        }
    }

    /// Move the selected weight index down (wraps).
    pub fn weight_down(&mut self) {
        let new_idx = self.selected_weight_idx + WEIGHT_COLS;
        if new_idx < CATEGORIES.len() {
            self.selected_weight_idx = new_idx;
        }
    }

    /// Move the selected weight index left.
    pub fn weight_left(&mut self) {
        if self.selected_weight_idx > 0 {
            self.selected_weight_idx -= 1;
        }
    }

    /// Move the selected weight index right.
    pub fn weight_right(&mut self) {
        if self.selected_weight_idx + 1 < CATEGORIES.len() {
            self.selected_weight_idx += 1;
        }
    }

    /// Start editing a numeric field.
    pub fn start_editing(&mut self, field_name: &str, current_value: &str) {
        self.editing_field = Some(field_name.to_string());
        self.field_input.set_value(current_value);
    }

    /// Confirm the current field edit and apply the value.
    ///
    /// Returns `true` if the edit was applied, `false` if the value was invalid.
    /// On invalid input the editing state is preserved so the user can retry.
    pub fn confirm_edit(&mut self) -> bool {
        let field = match &self.editing_field {
            Some(f) => f.clone(),
            None => return false,
        };

        if field == "budget" {
            if let Ok(val) = self.field_input.value().parse::<u8>() {
                if val <= 100 {
                    self.hitting_budget_pct = val;
                    self.editing_field = None;
                    self.field_input.clear();
                    return true;
                }
            }
            // Invalid input: preserve editing state so user can retry
            return false;
        }

        // Category weight field
        if let Ok(val) = self.field_input.value().parse::<f32>() {
            if val >= 0.0 && val <= 5.0 {
                if let Some(idx) = CATEGORIES.iter().position(|&c| c == field) {
                    self.category_weights.set(idx, val);
                    self.editing_field = None;
                    self.field_input.clear();
                    return true;
                }
            }
        }
        // Invalid input: preserve editing state so user can retry
        false
    }

    /// Cancel the current field edit.
    pub fn cancel_edit(&mut self) {
        self.editing_field = None;
        self.field_input.clear();
    }

    /// Check if any field is currently being edited.
    pub fn is_editing(&self) -> bool {
        self.editing_field.is_some() || self.ai_input_editing
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

/// Render the strategy setup screen into the given area.
pub fn render(frame: &mut Frame, area: Rect, state: &StrategySetupState) {
    // Outer block with title
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Line::from(vec![Span::styled(
            " Configure Your Draft Strategy ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]))
        .title_alignment(Alignment::Center);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Vertical layout sections
    let sections = Layout::vertical([
        Constraint::Length(1), // top padding
        Constraint::Length(1), // mode toggle
        Constraint::Length(1), // spacer
        Constraint::Length(6), // AI section (text area + button + status)
        Constraint::Length(1), // spacer
        Constraint::Length(1), // "Config Preview / Manual Edit" label
        Constraint::Length(1), // spacer
        Constraint::Length(1), // budget field
        Constraint::Length(1), // spacer
        Constraint::Length(1), // "Category Weights:" label
        Constraint::Length(4), // weight grid (4 rows of 3)
        Constraint::Min(0),   // flexible space
        Constraint::Length(1), // help bar
    ])
    .split(inner);

    // Horizontal centering
    let content_width = 56u16.min(inner.width);
    let h_offset = (inner.width.saturating_sub(content_width)) / 2;
    let content_rect = |row: Rect| -> Rect {
        Rect {
            x: row.x + h_offset,
            y: row.y,
            width: content_width.min(row.width.saturating_sub(h_offset)),
            height: row.height,
        }
    };

    // --- Mode toggle ---
    render_mode_toggle(frame, content_rect(sections[1]), state);

    // --- AI section ---
    if state.mode == StrategySetupMode::Ai {
        render_ai_section(frame, content_rect(sections[3]), state);
    }

    // --- Config preview label ---
    let label_text = if state.mode == StrategySetupMode::Ai {
        "Config Preview:"
    } else {
        "Manual Configuration:"
    };
    let label_style = Style::default().fg(Color::White);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(label_text, label_style))),
        content_rect(sections[5]),
    );

    // --- Budget field ---
    render_budget_field(frame, content_rect(sections[7]), state);

    // --- Category weights label ---
    let weights_active = state.active_section == StrategySection::CategoryWeights;
    let weights_label_style = if weights_active {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Category Weights:",
            weights_label_style,
        ))),
        content_rect(sections[9]),
    );

    // --- Weight grid ---
    render_weight_grid(frame, content_rect(sections[10]), state);

    // --- Help bar ---
    render_help_bar(frame, content_rect(sections[12]), state);
}

// ---------------------------------------------------------------------------
// Render helpers
// ---------------------------------------------------------------------------

/// Render the AI / Manual mode toggle.
fn render_mode_toggle(frame: &mut Frame, area: Rect, state: &StrategySetupState) {
    let active = state.active_section == StrategySection::ModeToggle;

    let ai_style = if state.mode == StrategySetupMode::Ai && active {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if state.mode == StrategySetupMode::Ai {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if active {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let manual_style = if state.mode == StrategySetupMode::Manual && active {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if state.mode == StrategySetupMode::Manual {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else if active {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let line = Line::from(vec![
        Span::styled("  [ Use AI to configure ]", ai_style),
        Span::styled("  ", Style::default()),
        Span::styled("[ Manual ]", manual_style),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

/// Render the AI input section (text area + generate button + status).
fn render_ai_section(frame: &mut Frame, area: Rect, state: &StrategySetupState) {
    let rows = Layout::vertical([
        Constraint::Length(1), // "Describe your strategy:" label
        Constraint::Length(3), // text area
        Constraint::Length(1), // generate button + status
        Constraint::Min(0),   // remaining
    ])
    .split(area);

    // Label
    let input_active = state.active_section == StrategySection::AiInput;
    let label_style = if input_active {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Describe your strategy:",
            label_style,
        ))),
        rows[0],
    );

    // Text area
    let border_style = if state.ai_input_editing {
        Style::default().fg(Color::Cyan)
    } else if input_active {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let ai_value = state.ai_input.value();
    let text_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style);

    let text_para = if state.ai_input_editing {
        // Show the cursor inline at the current cursor position.
        let cursor_char = state.ai_input.cursor_pos();
        let before: String = ai_value.chars().take(cursor_char).collect();
        let after: String = ai_value.chars().skip(cursor_char).collect();
        let text_style = Style::default().fg(Color::White);
        let cursor_style = Style::default().fg(Color::Black).bg(Color::Cyan);
        Paragraph::new(Line::from(vec![
            Span::styled(before, text_style),
            Span::styled("|", cursor_style),
            Span::styled(after, text_style),
        ]))
        .block(text_block)
        .wrap(Wrap { trim: false })
    } else if ai_value.is_empty() {
        Paragraph::new(Line::from(Span::styled(
            "Press Enter to type your strategy description...",
            Style::default().fg(Color::DarkGray),
        )))
        .block(text_block)
        .wrap(Wrap { trim: false })
    } else {
        Paragraph::new(Line::from(Span::styled(
            ai_value,
            Style::default().fg(Color::White),
        )))
        .block(text_block)
        .wrap(Wrap { trim: false })
    };

    frame.render_widget(text_para, rows[1]);

    // Generate button + status
    let btn_active = state.active_section == StrategySection::GenerateButton;
    let btn_style = if state.generating {
        Style::default().fg(Color::Yellow)
    } else if btn_active {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let btn_text = if state.generating {
        "[ Generating... ]"
    } else {
        "[ Generate Config ]"
    };

    let mut spans = vec![
        Span::styled("  ", Style::default()),
        Span::styled(btn_text, btn_style),
    ];

    // Status indicator
    if state.generating {
        spans.push(Span::styled(
            "  * Working...",
            Style::default().fg(Color::Yellow),
        ));
    } else if let Some(ref err) = state.generation_error {
        spans.push(Span::styled(
            format!("  x {}", err),
            Style::default().fg(Color::Red),
        ));
    } else if !state.generation_output.is_empty() {
        spans.push(Span::styled(
            "  * Config generated",
            Style::default().fg(Color::Green),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), rows[2]);
}

/// Render the budget field.
fn render_budget_field(frame: &mut Frame, area: Rect, state: &StrategySetupState) {
    let active = state.active_section == StrategySection::BudgetField;
    let editing = state.editing_field.as_deref() == Some("budget");

    let label_style = if active {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let value_style = if editing {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else if active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::Gray)
    };

    let line = if editing {
        // Show the cursor inline at the cursor position.
        let cursor_char = state.field_input.cursor_pos();
        let field_val = state.field_input.value();
        let before: String = field_val.chars().take(cursor_char).collect();
        let after: String = field_val.chars().skip(cursor_char).collect();
        let cursor_style = Style::default().fg(Color::Black).bg(Color::Cyan);
        Line::from(vec![
            Span::styled("  Budget (hitting %):     ", label_style),
            Span::styled("[ ", value_style),
            Span::styled(before, value_style),
            Span::styled("|", cursor_style),
            Span::styled(after, value_style),
            Span::styled(" ]", value_style),
        ])
    } else {
        Line::from(vec![
            Span::styled("  Budget (hitting %):     ", label_style),
            Span::styled(format!("[ {} ]", state.hitting_budget_pct), value_style),
        ])
    };
    frame.render_widget(Paragraph::new(line), area);
}

/// Render the 4x3 category weight grid.
fn render_weight_grid(frame: &mut Frame, area: Rect, state: &StrategySetupState) {
    let weights_active = state.active_section == StrategySection::CategoryWeights;
    let num_rows = (CATEGORIES.len() + WEIGHT_COLS - 1) / WEIGHT_COLS;

    for row in 0..num_rows {
        if row as u16 >= area.height {
            break;
        }
        let row_rect = Rect {
            x: area.x,
            y: area.y + row as u16,
            width: area.width,
            height: 1,
        };

        let mut spans = vec![Span::styled("  ", Style::default())];

        for col in 0..WEIGHT_COLS {
            let idx = row * WEIGHT_COLS + col;
            if idx >= CATEGORIES.len() {
                break;
            }

            let cat_name = CATEGORIES[idx];
            let is_selected = weights_active && idx == state.selected_weight_idx;
            let is_editing =
                is_selected && state.editing_field.as_deref() == Some(cat_name);

            let name_style = if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let value_str = if is_editing {
                let cursor_char = state.field_input.cursor_pos();
                let field_val = state.field_input.value();
                let before: String = field_val.chars().take(cursor_char).collect();
                let after: String = field_val.chars().skip(cursor_char).collect();
                format!("{}|{}", before, after)
            } else {
                format!("{:.1}", state.category_weights.get(idx))
            };

            let value_style = if is_editing {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };

            spans.push(Span::styled(
                format!("{:<4}", cat_name),
                name_style,
            ));
            spans.push(Span::styled(
                format!("[ {:<4}]", value_str),
                value_style,
            ));
            if col < WEIGHT_COLS - 1 {
                spans.push(Span::styled("  ", Style::default()));
            }
        }

        frame.render_widget(Paragraph::new(Line::from(spans)), row_rect);
    }
}

/// Render the help bar at the bottom of the screen.
fn render_help_bar(frame: &mut Frame, area: Rect, state: &StrategySetupState) {
    let help_spans = if state.ai_input_editing {
        vec![
            Span::styled("Type strategy", Style::default().fg(Color::Gray)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled("Enter:confirm", Style::default().fg(Color::Gray)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled("Esc:cancel", Style::default().fg(Color::Gray)),
        ]
    } else if state.editing_field.is_some() {
        vec![
            Span::styled("Type value", Style::default().fg(Color::Gray)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled("Enter:confirm", Style::default().fg(Color::Gray)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled("Esc:cancel", Style::default().fg(Color::Gray)),
        ]
    } else {
        vec![
            Span::styled("Tab:section", Style::default().fg(Color::Gray)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled("^v:navigate", Style::default().fg(Color::Gray)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled("Enter:edit", Style::default().fg(Color::Gray)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled("s:save", Style::default().fg(Color::Gray)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled("Esc:back", Style::default().fg(Color::Gray)),
        ]
    };

    frame.render_widget(
        Paragraph::new(Line::from(help_spans)).alignment(Alignment::Center),
        area,
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- CategoryWeights tests --

    #[test]
    fn default_weights() {
        let w = CategoryWeights::default();
        assert!((w.r - 1.0).abs() < f32::EPSILON);
        assert!((w.sv - 0.7).abs() < f32::EPSILON);
        assert!((w.hd - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn get_set_roundtrip() {
        let mut w = CategoryWeights::default();
        w.set(3, 1.3); // BB
        assert!((w.get(3) - 1.3).abs() < f32::EPSILON);
        assert_eq!(w.bb, 1.3);

        w.set(8, 0.3); // SV
        assert!((w.get(8) - 0.3).abs() < f32::EPSILON);
        assert_eq!(w.sv, 0.3);
    }

    #[test]
    fn get_out_of_bounds_returns_default() {
        let w = CategoryWeights::default();
        assert!((w.get(99) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn set_out_of_bounds_is_noop() {
        let mut w = CategoryWeights::default();
        w.set(99, 5.0);
        // No crash, values unchanged
        assert!((w.r - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn to_from_config_roundtrip() {
        let w = CategoryWeights {
            r: 1.0,
            hr: 1.1,
            rbi: 0.9,
            bb: 1.3,
            sb: 1.0,
            avg: 1.0,
            k: 1.0,
            w: 1.0,
            sv: 0.3,
            hd: 1.2,
            era: 1.0,
            whip: 1.0,
        };
        let config_w = w.to_config_weights();
        let back = CategoryWeights::from_config_weights(&config_w);
        // Compare within f32 precision (f64->f32 loses some precision)
        for i in 0..12 {
            assert!(
                (w.get(i) - back.get(i)).abs() < 0.001,
                "mismatch at index {}: {} vs {}",
                i,
                w.get(i),
                back.get(i)
            );
        }
    }

    // -- StrategySetupState tests --

    #[test]
    fn default_state() {
        let s = StrategySetupState::default();
        assert_eq!(s.mode, StrategySetupMode::Ai);
        assert_eq!(s.active_section, StrategySection::ModeToggle);
        assert_eq!(s.hitting_budget_pct, 65);
        assert!(!s.generating);
        assert!(!s.ai_input_editing);
        assert!(s.editing_field.is_none());
    }

    #[test]
    fn toggle_mode() {
        let mut s = StrategySetupState::default();
        s.toggle_mode();
        assert_eq!(s.mode, StrategySetupMode::Manual);
        s.toggle_mode();
        assert_eq!(s.mode, StrategySetupMode::Ai);
    }

    #[test]
    fn toggle_mode_adjusts_section() {
        let mut s = StrategySetupState::default();
        s.active_section = StrategySection::AiInput;
        s.toggle_mode(); // to Manual
        // AiInput is not in Manual cycle, should be adjusted
        assert_eq!(s.active_section, StrategySection::BudgetField);
    }

    #[test]
    fn toggle_mode_preserves_valid_section() {
        let mut s = StrategySetupState::default();
        s.active_section = StrategySection::CategoryWeights;
        s.toggle_mode(); // to Manual
        assert_eq!(s.active_section, StrategySection::CategoryWeights);
    }

    #[test]
    fn section_next_ai_mode() {
        let s = StrategySection::ModeToggle;
        assert_eq!(s.next(StrategySetupMode::Ai), StrategySection::AiInput);
        assert_eq!(
            s.next(StrategySetupMode::Ai)
                .next(StrategySetupMode::Ai),
            StrategySection::GenerateButton
        );
    }

    #[test]
    fn section_next_manual_mode() {
        let s = StrategySection::ModeToggle;
        assert_eq!(
            s.next(StrategySetupMode::Manual),
            StrategySection::BudgetField
        );
    }

    #[test]
    fn section_wraps() {
        let s = StrategySection::CategoryWeights;
        assert_eq!(s.next(StrategySetupMode::Ai), StrategySection::ModeToggle);
        assert_eq!(
            s.next(StrategySetupMode::Manual),
            StrategySection::ModeToggle
        );
    }

    #[test]
    fn section_prev_wraps() {
        let s = StrategySection::ModeToggle;
        assert_eq!(
            s.prev(StrategySetupMode::Ai),
            StrategySection::CategoryWeights
        );
        assert_eq!(
            s.prev(StrategySetupMode::Manual),
            StrategySection::CategoryWeights
        );
    }

    #[test]
    fn weight_navigation() {
        let mut s = StrategySetupState::default();
        assert_eq!(s.selected_weight_idx, 0);

        s.weight_right();
        assert_eq!(s.selected_weight_idx, 1);

        s.weight_down();
        assert_eq!(s.selected_weight_idx, 4); // 1 + WEIGHT_COLS(3) = 4

        s.weight_left();
        assert_eq!(s.selected_weight_idx, 3);

        s.weight_up();
        assert_eq!(s.selected_weight_idx, 0);
    }

    #[test]
    fn weight_navigation_bounds() {
        let mut s = StrategySetupState::default();
        s.weight_left(); // Already at 0, should stay
        assert_eq!(s.selected_weight_idx, 0);

        s.weight_up(); // Already at top, should stay
        assert_eq!(s.selected_weight_idx, 0);

        s.selected_weight_idx = 11; // Last index
        s.weight_right(); // At end, should stay
        assert_eq!(s.selected_weight_idx, 11);

        s.weight_down(); // Would go past end, should stay
        assert_eq!(s.selected_weight_idx, 11);
    }

    #[test]
    fn edit_budget_field() {
        let mut s = StrategySetupState::default();
        s.start_editing("budget", "65");
        assert_eq!(s.editing_field.as_deref(), Some("budget"));
        assert_eq!(s.field_input.value(), "65");

        s.field_input.set_value("70");
        assert!(s.confirm_edit());
        assert_eq!(s.hitting_budget_pct, 70);
        assert!(s.editing_field.is_none());
    }

    #[test]
    fn edit_budget_rejects_over_100() {
        let mut s = StrategySetupState::default();
        s.start_editing("budget", "65");
        s.field_input.set_value("101");
        assert!(!s.confirm_edit());
        assert_eq!(s.hitting_budget_pct, 65); // unchanged
        // Editing state should be preserved so user can retry
        assert_eq!(s.editing_field.as_deref(), Some("budget"));
        assert_eq!(s.field_input.value(), "101");
    }

    #[test]
    fn edit_weight_field() {
        let mut s = StrategySetupState::default();
        s.start_editing("BB", "1.0");
        s.field_input.set_value("1.3");
        assert!(s.confirm_edit());
        assert!((s.category_weights.bb - 1.3).abs() < f32::EPSILON);
    }

    #[test]
    fn edit_weight_accepts_zero() {
        let mut s = StrategySetupState::default();
        s.start_editing("SV", "0.7");
        s.field_input.set_value("0.0");
        assert!(s.confirm_edit());
        assert!((s.category_weights.sv - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn edit_weight_rejects_negative() {
        let mut s = StrategySetupState::default();
        s.start_editing("SV", "0.7");
        s.field_input.set_value("-0.1");
        assert!(!s.confirm_edit());
        assert!((s.category_weights.sv - 0.7).abs() < f32::EPSILON);
        // Editing state should be preserved so user can retry
        assert_eq!(s.editing_field.as_deref(), Some("SV"));
    }

    #[test]
    fn edit_weight_rejects_over_5() {
        let mut s = StrategySetupState::default();
        s.start_editing("R", "1.0");
        s.field_input.set_value("5.1");
        assert!(!s.confirm_edit());
        // Editing state should be preserved so user can retry
        assert_eq!(s.editing_field.as_deref(), Some("R"));
    }

    #[test]
    fn cancel_edit() {
        let mut s = StrategySetupState::default();
        s.start_editing("budget", "65");
        s.field_input.set_value("99");
        s.cancel_edit();
        assert!(s.editing_field.is_none());
        assert_eq!(s.hitting_budget_pct, 65); // unchanged
    }

    #[test]
    fn is_editing_checks() {
        let mut s = StrategySetupState::default();
        assert!(!s.is_editing());

        s.ai_input_editing = true;
        assert!(s.is_editing());

        s.ai_input_editing = false;
        s.editing_field = Some("budget".to_string());
        assert!(s.is_editing());
    }

    // -- Render tests --

    #[test]
    fn render_default_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = StrategySetupState::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_manual_mode_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = StrategySetupState::default();
        state.mode = StrategySetupMode::Manual;
        state.active_section = StrategySection::CategoryWeights;
        state.selected_weight_idx = 5;
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_editing_mode_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = StrategySetupState::default();
        state.editing_field = Some("budget".to_string());
        state.field_input.set_value("70");
        state.active_section = StrategySection::BudgetField;
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_ai_generating_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = StrategySetupState::default();
        state.generating = true;
        state.active_section = StrategySection::GenerateButton;
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_ai_input_editing_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = StrategySetupState::default();
        state.ai_input_editing = true;
        state.ai_input.set_value("punt saves, go heavy K and BB");
        state.active_section = StrategySection::AiInput;
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_with_generation_error() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = StrategySetupState::default();
        state.generation_error = Some("LLM disabled".to_string());
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_small_terminal_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(40, 15);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = StrategySetupState::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }
}
