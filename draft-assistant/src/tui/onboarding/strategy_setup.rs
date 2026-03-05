// Strategy setup screen: linear wizard flow for draft strategy configuration.
//
// This is Step 2 of the onboarding wizard. The wizard proceeds through four
// steps:
//   1. Input: large text area for natural language strategy description
//   2. Generating: LLM streams output while processing the description
//   3. Review: shows LLM-generated overview + budget/weights for inline editing
//   4. Confirm: "Save this draft strategy?" Yes/No prompt
//
// When accessed from the Settings screen (after onboarding is complete), the
// wizard shows the Review step directly with all values editable.

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
pub const WEIGHT_COLS: usize = 3;

// ---------------------------------------------------------------------------
// StrategyWizardStep
// ---------------------------------------------------------------------------

/// Which step of the strategy wizard is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyWizardStep {
    /// Step 1: user describes strategy in a large text input.
    Input,
    /// Step 2: LLM is generating config from the description.
    Generating,
    /// Step 3: review generated values (overview + budget + weights).
    Review,
    /// Step 4: "Save this draft strategy?" confirmation.
    Confirm,
}

// ---------------------------------------------------------------------------
// ReviewSection
// ---------------------------------------------------------------------------

/// Which part of the review step has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewSection {
    /// Strategy overview text (editable, scrollable).
    Overview,
    /// Hitting budget percentage field.
    BudgetField,
    /// Category weight grid.
    CategoryWeights,
}

impl ReviewSection {
    /// Ordered list of sections for Tab cycling in review mode.
    const CYCLE: &[ReviewSection] = &[
        ReviewSection::Overview,
        ReviewSection::BudgetField,
        ReviewSection::CategoryWeights,
    ];

    /// Advance to the next section (wraps around).
    pub fn next(self) -> ReviewSection {
        let idx = Self::CYCLE.iter().position(|&s| s == self).unwrap_or(0);
        Self::CYCLE[(idx + 1) % Self::CYCLE.len()]
    }

    /// Go to the previous section (wraps around).
    pub fn prev(self) -> ReviewSection {
        let idx = Self::CYCLE.iter().position(|&s| s == self).unwrap_or(0);
        if idx == 0 {
            Self::CYCLE[Self::CYCLE.len() - 1]
        } else {
            Self::CYCLE[idx - 1]
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

/// UI state for the strategy setup wizard.
///
/// Lives inside `ViewState` so the TUI can render it without any global state.
#[derive(Debug, Clone)]
pub struct StrategySetupState {
    /// Current wizard step.
    pub step: StrategyWizardStep,
    /// Text area content for strategy description (with cursor tracking).
    pub strategy_input: TextInput,
    /// Whether the strategy text input is in edit mode (cursor active).
    pub input_editing: bool,
    /// Whether the LLM is currently generating.
    pub generating: bool,
    /// Streamed LLM output text (shown during Generating step).
    pub generation_output: String,
    /// Error message from LLM generation, if any.
    pub generation_error: Option<String>,
    /// Hitting budget percentage (0-100).
    pub hitting_budget_pct: u8,
    /// Category weight values.
    pub category_weights: CategoryWeights,
    /// LLM-generated prose overview of the strategy.
    pub strategy_overview: String,
    /// Which field is being edited in Review step (None = not editing,
    /// Some("budget") or Some(category name)).
    pub editing_field: Option<String>,
    /// Current text being typed in an editable numeric field (with cursor tracking).
    pub field_input: TextInput,
    /// Which category weight is highlighted (0-11).
    pub selected_weight_idx: usize,
    /// Which part of the Review step has focus.
    pub review_section: ReviewSection,
    /// Whether the confirm prompt is selecting "Yes" (true) or "No" (false).
    pub confirm_yes: bool,
    /// Whether the strategy overview text box is being edited in review mode.
    pub overview_editing: bool,
    /// Text input buffer for editing the strategy overview.
    pub overview_input: TextInput,
    /// Whether strategy settings have been modified since last save.
    pub settings_dirty: bool,
    /// Snapshot of strategy overview for Esc restore in settings mode.
    pub snapshot_overview: String,
    /// Snapshot of hitting budget percentage for Esc restore in settings mode.
    pub snapshot_budget: u8,
    /// Snapshot of category weights for Esc restore in settings mode.
    pub snapshot_weights: CategoryWeights,
}

impl Default for StrategySetupState {
    fn default() -> Self {
        StrategySetupState {
            step: StrategyWizardStep::Input,
            strategy_input: TextInput::new(),
            input_editing: true, // auto-focused in edit mode
            generating: false,
            generation_output: String::new(),
            generation_error: None,
            hitting_budget_pct: 65,
            category_weights: CategoryWeights::default(),
            strategy_overview: String::new(),
            editing_field: None,
            field_input: TextInput::new(),
            selected_weight_idx: 0,
            review_section: ReviewSection::Overview,
            confirm_yes: true,
            overview_editing: false,
            overview_input: TextInput::new(),
            settings_dirty: false,
            snapshot_overview: String::new(),
            snapshot_budget: 65,
            snapshot_weights: CategoryWeights::default(),
        }
    }
}

impl StrategySetupState {
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
        self.editing_field.is_some() || self.input_editing || self.overview_editing
    }

    /// Start editing the strategy overview text.
    ///
    /// Copies the current overview into the text input buffer and activates
    /// editing mode.
    pub fn start_overview_editing(&mut self) {
        self.overview_input.set_value(&self.strategy_overview);
        self.overview_editing = true;
    }

    /// Cancel overview editing and discard changes to the text input.
    pub fn cancel_overview_editing(&mut self) {
        self.overview_editing = false;
        self.overview_input.clear();
    }

    /// Snapshot the current strategy values for Esc restoration in settings mode.
    pub fn snapshot_settings(&mut self) {
        self.snapshot_overview = self.strategy_overview.clone();
        self.snapshot_budget = self.hitting_budget_pct;
        self.snapshot_weights = self.category_weights.clone();
    }

    /// Restore strategy values from the last snapshot (undo unsaved changes).
    pub fn restore_settings_snapshot(&mut self) {
        self.strategy_overview = self.snapshot_overview.clone();
        self.hitting_budget_pct = self.snapshot_budget;
        self.category_weights = self.snapshot_weights.clone();
        self.settings_dirty = false;
        self.overview_editing = false;
        self.overview_input.clear();
        self.editing_field = None;
        self.field_input.clear();
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

/// Render the strategy setup wizard into the given area.
pub fn render(frame: &mut Frame, area: Rect, state: &StrategySetupState) {
    match state.step {
        StrategyWizardStep::Input => render_input_step(frame, area, state),
        StrategyWizardStep::Generating => render_generating_step(frame, area, state),
        StrategyWizardStep::Review => render_review_step(frame, area, state),
        StrategyWizardStep::Confirm => render_confirm_step(frame, area, state),
    }
}

// ---------------------------------------------------------------------------
// Step 1: Input
// ---------------------------------------------------------------------------

fn render_input_step(frame: &mut Frame, area: Rect, state: &StrategySetupState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Line::from(vec![Span::styled(
            " Describe Your Draft Strategy ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]))
        .title_alignment(Alignment::Center);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = Layout::vertical([
        Constraint::Length(1), // top padding
        Constraint::Length(2), // instructions
        Constraint::Length(1), // spacer
        Constraint::Min(6),   // text area (fills remaining space)
        Constraint::Length(1), // spacer
        Constraint::Length(1), // help bar
    ])
    .split(inner);

    // Horizontal centering
    let content_width = 70u16.min(inner.width);
    let h_offset = (inner.width.saturating_sub(content_width)) / 2;
    let content_rect = |row: Rect| -> Rect {
        Rect {
            x: row.x + h_offset,
            y: row.y,
            width: content_width.min(row.width.saturating_sub(h_offset)),
            height: row.height,
        }
    };

    // Instructions
    let instructions = Paragraph::new(vec![
        Line::from(Span::styled(
            "Describe your draft strategy in plain English below.",
            Style::default().fg(Color::White),
        )),
        Line::from(Span::styled(
            "The AI will generate budget split, category weights, and a strategy overview.",
            Style::default().fg(Color::DarkGray),
        )),
    ]);
    frame.render_widget(instructions, content_rect(sections[1]));

    // Text area
    let border_style = if state.input_editing {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::White)
    };

    let input_value = state.strategy_input.value();
    let text_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style);

    let text_para = if state.input_editing {
        let cursor_char = state.strategy_input.cursor_pos();
        let before: String = input_value.chars().take(cursor_char).collect();
        let after: String = input_value.chars().skip(cursor_char).collect();
        let text_style = Style::default().fg(Color::White);
        let cursor_style = Style::default().fg(Color::Black).bg(Color::Cyan);
        Paragraph::new(Line::from(vec![
            Span::styled(before, text_style),
            Span::styled("|", cursor_style),
            Span::styled(after, text_style),
        ]))
        .block(text_block)
        .wrap(Wrap { trim: false })
    } else if input_value.is_empty() {
        Paragraph::new(Line::from(Span::styled(
            "e.g. Stars-and-scrubs, punt saves, heavy on BB and HD...",
            Style::default().fg(Color::DarkGray),
        )))
        .block(text_block)
        .wrap(Wrap { trim: false })
    } else {
        Paragraph::new(Line::from(Span::styled(
            input_value,
            Style::default().fg(Color::White),
        )))
        .block(text_block)
        .wrap(Wrap { trim: false })
    };

    frame.render_widget(text_para, content_rect(sections[3]));

    // Error message if present (from a previous failed generation)
    if let Some(ref err) = state.generation_error {
        // Overlay error on the last line of the text area
        let err_line = Line::from(Span::styled(
            format!("  Error: {}", err),
            Style::default().fg(Color::Red),
        ));
        let err_area = content_rect(sections[4]);
        frame.render_widget(Paragraph::new(err_line), err_area);
    }

    // Help bar
    let help_spans = if state.input_editing {
        vec![
            Span::styled("Type strategy", Style::default().fg(Color::Gray)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled("Esc:stop editing", Style::default().fg(Color::Gray)),
        ]
    } else {
        vec![
            Span::styled("Enter:generate", Style::default().fg(Color::Gray)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled("e:edit", Style::default().fg(Color::Gray)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled("Esc:back", Style::default().fg(Color::Gray)),
        ]
    };
    frame.render_widget(
        Paragraph::new(Line::from(help_spans)).alignment(Alignment::Center),
        content_rect(sections[5]),
    );
}

// ---------------------------------------------------------------------------
// Step 2: Generating
// ---------------------------------------------------------------------------

fn render_generating_step(frame: &mut Frame, area: Rect, state: &StrategySetupState) {
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

    if state.generation_error.is_some() {
        // Error state: show error message centered with retry/back options
        let sections = Layout::vertical([
            Constraint::Min(1),    // top spacer
            Constraint::Length(1), // error icon
            Constraint::Length(1), // spacer
            Constraint::Length(2), // error message
            Constraint::Min(1),    // bottom spacer
            Constraint::Length(1), // help bar
        ])
        .split(inner);

        let content_width = 60u16.min(inner.width);
        let h_offset = (inner.width.saturating_sub(content_width)) / 2;
        let content_rect = |row: Rect| -> Rect {
            Rect {
                x: row.x + h_offset,
                y: row.y,
                width: content_width.min(row.width.saturating_sub(h_offset)),
                height: row.height,
            }
        };

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Generation failed",
                Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            )))
            .alignment(Alignment::Center),
            content_rect(sections[1]),
        );

        if let Some(ref err) = state.generation_error {
            frame.render_widget(
                Paragraph::new(Span::styled(err.as_str(), Style::default().fg(Color::Red)))
                    .alignment(Alignment::Center)
                    .wrap(Wrap { trim: false }),
                content_rect(sections[3]),
            );
        }

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Enter:retry", Style::default().fg(Color::Gray)),
                Span::styled(" | ", Style::default().fg(Color::DarkGray)),
                Span::styled("Esc:back to input", Style::default().fg(Color::Gray)),
            ]))
            .alignment(Alignment::Center),
            content_rect(sections[5]),
        );
    } else {
        // Generating: centered "Thinking..." with a simple animation
        let sections = Layout::vertical([
            Constraint::Min(1),    // top spacer
            Constraint::Length(1), // thinking text
            Constraint::Length(1), // dots
            Constraint::Min(1),    // bottom spacer
        ])
        .split(inner);

        let content_width = 40u16.min(inner.width);
        let h_offset = (inner.width.saturating_sub(content_width)) / 2;
        let content_rect = |row: Rect| -> Rect {
            Rect {
                x: row.x + h_offset,
                y: row.y,
                width: content_width.min(row.width.saturating_sub(h_offset)),
                height: row.height,
            }
        };

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Thinking...",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )))
            .alignment(Alignment::Center),
            content_rect(sections[1]),
        );

        // Subtle animated dots based on output length (each token advances the animation)
        let dot_count = (state.generation_output.len() / 20) % 4;
        let dots = ".".repeat(dot_count);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                dots,
                Style::default().fg(Color::DarkGray),
            )))
            .alignment(Alignment::Center),
            content_rect(sections[2]),
        );

    }
}

// ---------------------------------------------------------------------------
// Step 3: Review
// ---------------------------------------------------------------------------

fn render_review_step(frame: &mut Frame, area: Rect, state: &StrategySetupState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Line::from(vec![Span::styled(
            " Review Your Strategy ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]))
        .title_alignment(Alignment::Center);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Keybind hints are shown exclusively in the app-level bottom help bar
    // (see compute_settings_keybinds / compute_onboarding_keybinds in tui/mod.rs).
    let sections = Layout::vertical([
        Constraint::Length(1),  // top padding
        Constraint::Length(1),  // "Strategy Overview:" label
        Constraint::Min(6),    // overview text (fills available space)
        Constraint::Length(1),  // spacer
        Constraint::Length(1),  // budget field
        Constraint::Length(1),  // spacer
        Constraint::Length(1),  // "Category Weights:" label
        Constraint::Length(4),  // weight grid (4 rows of 3)
    ])
    .split(inner);

    let content_width = 70u16.min(inner.width);
    let h_offset = (inner.width.saturating_sub(content_width)) / 2;
    let content_rect = |row: Rect| -> Rect {
        Rect {
            x: row.x + h_offset,
            y: row.y,
            width: content_width.min(row.width.saturating_sub(h_offset)),
            height: row.height,
        }
    };

    // --- Strategy Overview ---
    let overview_active = state.review_section == ReviewSection::Overview;
    let overview_label_style = if overview_active {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    // Show status badge next to label
    let label_spans = if state.generating {
        vec![
            Span::styled("Strategy Overview:  ", overview_label_style),
            Span::styled(
                "Thinking...",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]
    } else if state.generation_error.is_some() {
        vec![
            Span::styled("Strategy Overview:  ", overview_label_style),
            Span::styled(
                "[error]",
                Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ),
        ]
    } else if state.overview_editing {
        vec![
            Span::styled("Strategy Overview:  ", overview_label_style),
            Span::styled(
                "[editing]",
                Style::default().fg(Color::Cyan),
            ),
        ]
    } else {
        vec![Span::styled("Strategy Overview:", overview_label_style)]
    };
    frame.render_widget(
        Paragraph::new(Line::from(label_spans)),
        content_rect(sections[1]),
    );

    let overview_border = if state.overview_editing {
        Style::default().fg(Color::Yellow)
    } else if overview_active {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let overview_block = Block::default()
        .borders(Borders::ALL)
        .border_style(overview_border);

    if state.overview_editing {
        // Editable text input with cursor
        let input_value = state.overview_input.value();
        let cursor_char = state.overview_input.cursor_pos();
        let before: String = input_value.chars().take(cursor_char).collect();
        let after: String = input_value.chars().skip(cursor_char).collect();
        let text_style = Style::default().fg(Color::White);
        let cursor_style = Style::default().fg(Color::Black).bg(Color::Yellow);
        let overview_para = Paragraph::new(Line::from(vec![
            Span::styled(before, text_style),
            Span::styled("|", cursor_style),
            Span::styled(after, text_style),
        ]))
        .block(overview_block)
        .wrap(Wrap { trim: false });
        frame.render_widget(overview_para, content_rect(sections[2]));
    } else if state.generating {
        // Generating: show animated dots in the overview area
        let dot_count = (state.generation_output.len() / 20) % 4;
        let dots = ".".repeat(dot_count);
        let overview_para = Paragraph::new(Span::styled(
            dots,
            Style::default().fg(Color::DarkGray),
        ))
        .block(overview_block)
        .wrap(Wrap { trim: false });
        frame.render_widget(overview_para, content_rect(sections[2]));
    } else if let Some(ref err) = state.generation_error {
        // Error state: show error message in red
        let error_lines = vec![
            Line::from(Span::styled(
                format!("Error: {err}"),
                Style::default().fg(Color::Red),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Esc: back to editing | Enter: retry",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        let overview_para = Paragraph::new(error_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Red)),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(overview_para, content_rect(sections[2]));
    } else {
        let overview_text = if state.strategy_overview.is_empty() {
            "(No overview generated)"
        } else {
            &state.strategy_overview
        };
        let overview_para = Paragraph::new(Span::styled(
            overview_text,
            Style::default().fg(Color::White),
        ))
        .block(overview_block)
        .wrap(Wrap { trim: false });
        frame.render_widget(overview_para, content_rect(sections[2]));
    };

    // --- Budget field ---
    render_budget_field(frame, content_rect(sections[4]), state);

    // --- Category weights label ---
    let weights_active = state.review_section == ReviewSection::CategoryWeights;
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
        content_rect(sections[6]),
    );

    // --- Weight grid ---
    render_weight_grid(frame, content_rect(sections[7]), state);
}

// ---------------------------------------------------------------------------
// Step 4: Confirm
// ---------------------------------------------------------------------------

fn render_confirm_step(frame: &mut Frame, area: Rect, state: &StrategySetupState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Line::from(vec![Span::styled(
            " Save Strategy ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]))
        .title_alignment(Alignment::Center);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = Layout::vertical([
        Constraint::Min(1),    // centering space
        Constraint::Length(1), // question
        Constraint::Length(2), // spacer
        Constraint::Length(1), // yes/no buttons
        Constraint::Min(1),    // centering space
        Constraint::Length(1), // help bar
    ])
    .split(inner);

    let content_width = 50u16.min(inner.width);
    let h_offset = (inner.width.saturating_sub(content_width)) / 2;
    let content_rect = |row: Rect| -> Rect {
        Rect {
            x: row.x + h_offset,
            y: row.y,
            width: content_width.min(row.width.saturating_sub(h_offset)),
            height: row.height,
        }
    };

    // Question
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Save this draft strategy?",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center),
        content_rect(sections[1]),
    );

    // Yes/No buttons
    let yes_style = if state.confirm_yes {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Green)
    };

    let no_style = if !state.confirm_yes {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Red)
    };

    let buttons = Line::from(vec![
        Span::styled("  [ Yes, save and enter draft ]", yes_style),
        Span::styled("    ", Style::default()),
        Span::styled("[ No, go back ]", no_style),
    ]);
    frame.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        content_rect(sections[3]),
    );

    // Help bar
    let help_spans = vec![
        Span::styled("<>:select", Style::default().fg(Color::Gray)),
        Span::styled(" | ", Style::default().fg(Color::DarkGray)),
        Span::styled("Enter:confirm", Style::default().fg(Color::Gray)),
        Span::styled(" | ", Style::default().fg(Color::DarkGray)),
        Span::styled("Esc:back", Style::default().fg(Color::Gray)),
    ];
    frame.render_widget(
        Paragraph::new(Line::from(help_spans)).alignment(Alignment::Center),
        content_rect(sections[5]),
    );
}

// ---------------------------------------------------------------------------
// Render helpers (shared across steps)
// ---------------------------------------------------------------------------

/// Render the budget field.
fn render_budget_field(frame: &mut Frame, area: Rect, state: &StrategySetupState) {
    let active = state.review_section == ReviewSection::BudgetField;
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
    let weights_active = state.review_section == ReviewSection::CategoryWeights;
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
        assert_eq!(s.step, StrategyWizardStep::Input);
        assert_eq!(s.hitting_budget_pct, 65);
        assert!(!s.generating);
        assert!(s.input_editing); // auto-focused
        assert!(s.editing_field.is_none());
        assert_eq!(s.review_section, ReviewSection::Overview);
        assert!(s.confirm_yes);
    }

    #[test]
    fn review_section_next_wraps() {
        let s = ReviewSection::CategoryWeights;
        assert_eq!(s.next(), ReviewSection::Overview);
    }

    #[test]
    fn review_section_prev_wraps() {
        let s = ReviewSection::Overview;
        assert_eq!(s.prev(), ReviewSection::CategoryWeights);
    }

    #[test]
    fn review_section_next_cycle() {
        let s = ReviewSection::Overview;
        assert_eq!(s.next(), ReviewSection::BudgetField);
        assert_eq!(s.next().next(), ReviewSection::CategoryWeights);
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
        // Default has input_editing = true
        assert!(s.is_editing());

        s.input_editing = false;
        assert!(!s.is_editing());

        s.editing_field = Some("budget".to_string());
        assert!(s.is_editing());
    }

    // -- Render tests --

    #[test]
    fn render_input_step_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = StrategySetupState::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_generating_step_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = StrategySetupState::default();
        state.step = StrategyWizardStep::Generating;
        state.generating = true;
        state.generation_output = "Processing...".to_string();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_review_step_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = StrategySetupState::default();
        state.step = StrategyWizardStep::Review;
        state.strategy_overview = "Stars-and-scrubs approach.".to_string();
        state.review_section = ReviewSection::BudgetField;
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_confirm_step_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = StrategySetupState::default();
        state.step = StrategyWizardStep::Confirm;
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_review_editing_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = StrategySetupState::default();
        state.step = StrategyWizardStep::Review;
        state.editing_field = Some("budget".to_string());
        state.field_input.set_value("70");
        state.review_section = ReviewSection::BudgetField;
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_generating_error_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = StrategySetupState::default();
        state.step = StrategyWizardStep::Generating;
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

    // -- Overview editing tests --

    #[test]
    fn start_overview_editing_copies_text() {
        let mut s = StrategySetupState::default();
        s.strategy_overview = "Stars-and-scrubs approach.".to_string();
        s.start_overview_editing();
        assert!(s.overview_editing);
        assert_eq!(s.overview_input.value(), "Stars-and-scrubs approach.");
    }

    #[test]
    fn cancel_overview_editing_clears_input() {
        let mut s = StrategySetupState::default();
        s.strategy_overview = "Original overview.".to_string();
        s.start_overview_editing();
        s.overview_input.set_value("Modified text");
        s.cancel_overview_editing();
        assert!(!s.overview_editing);
        assert!(s.overview_input.is_empty());
        // Original overview is unchanged
        assert_eq!(s.strategy_overview, "Original overview.");
    }

    #[test]
    fn is_editing_includes_overview_editing() {
        let mut s = StrategySetupState::default();
        s.input_editing = false;
        assert!(!s.is_editing());

        s.overview_editing = true;
        assert!(s.is_editing());
    }

    // -- Settings snapshot tests --

    #[test]
    fn snapshot_and_restore_settings() {
        let mut s = StrategySetupState::default();
        s.strategy_overview = "Original".to_string();
        s.hitting_budget_pct = 65;
        s.category_weights.bb = 1.3;
        s.snapshot_settings();

        // Modify values
        s.strategy_overview = "Modified".to_string();
        s.hitting_budget_pct = 80;
        s.category_weights.bb = 2.0;
        s.settings_dirty = true;
        s.overview_editing = true;

        // Restore
        s.restore_settings_snapshot();
        assert_eq!(s.strategy_overview, "Original");
        assert_eq!(s.hitting_budget_pct, 65);
        assert!((s.category_weights.bb - 1.3).abs() < f32::EPSILON);
        assert!(!s.settings_dirty);
        assert!(!s.overview_editing);
    }

    // -- Render tests for new states --

    #[test]
    fn render_review_overview_editing_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = StrategySetupState::default();
        state.step = StrategyWizardStep::Review;
        state.overview_editing = true;
        state.overview_input.set_value("Editing this strategy text");
        state.review_section = ReviewSection::Overview;
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_review_generating_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = StrategySetupState::default();
        state.step = StrategyWizardStep::Review;
        state.generating = true;
        state.generation_output = "Processing tokens...".to_string();
        state.review_section = ReviewSection::Overview;
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }
}
