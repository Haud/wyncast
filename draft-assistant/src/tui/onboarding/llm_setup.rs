// LLM setup screen: provider, model, and API key configuration.
//
// This is Step 1 of the onboarding wizard. The user selects an LLM provider,
// chooses a model, enters an API key, and optionally tests the connection
// before proceeding to strategy configuration.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::llm::provider::{models_for_provider, LlmProvider, ModelOption, SUPPORTED_MODELS};

// ---------------------------------------------------------------------------
// LlmSetupSection
// ---------------------------------------------------------------------------

/// Which section of the LLM setup screen currently has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmSetupSection {
    Provider,
    Model,
    ApiKey,
    TestButton,
}

impl LlmSetupSection {
    /// Ordered list of sections for Tab cycling.
    const CYCLE: &[LlmSetupSection] = &[
        LlmSetupSection::Provider,
        LlmSetupSection::Model,
        LlmSetupSection::ApiKey,
        LlmSetupSection::TestButton,
    ];

    /// Advance to the next section (wraps around).
    pub fn next(self) -> LlmSetupSection {
        let idx = Self::CYCLE.iter().position(|&s| s == self).unwrap_or(0);
        Self::CYCLE[(idx + 1) % Self::CYCLE.len()]
    }

    /// Go to the previous section (wraps around).
    pub fn prev(self) -> LlmSetupSection {
        let idx = Self::CYCLE.iter().position(|&s| s == self).unwrap_or(0);
        if idx == 0 {
            Self::CYCLE[Self::CYCLE.len() - 1]
        } else {
            Self::CYCLE[idx - 1]
        }
    }
}

// ---------------------------------------------------------------------------
// LlmConnectionStatus
// ---------------------------------------------------------------------------

/// Status of the API connection test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmConnectionStatus {
    /// No test has been run yet.
    Untested,
    /// A test is currently in progress.
    Testing,
    /// The test succeeded.
    Success(String),
    /// The test failed with an error message.
    Failed(String),
}

impl Default for LlmConnectionStatus {
    fn default() -> Self {
        LlmConnectionStatus::Untested
    }
}

// ---------------------------------------------------------------------------
// LlmSetupState
// ---------------------------------------------------------------------------

/// UI state for the LLM setup screen.
///
/// Lives inside `ViewState` so the TUI can render it without any global state.
///
/// Note: custom `Debug` implementation redacts `api_key_input` and
/// `api_key_backup` to avoid leaking secrets in log output.
#[derive(Clone)]
pub struct LlmSetupState {
    /// Which section currently has keyboard focus.
    pub active_section: LlmSetupSection,
    /// Index into the provider list (Anthropic=0, Google=1, OpenAI=2).
    pub selected_provider_idx: usize,
    /// Index into the model list for the currently selected provider.
    pub selected_model_idx: usize,
    /// The API key text as entered by the user.
    pub api_key_input: String,
    /// Backup of the API key before entering edit mode, restored on Esc cancel.
    pub api_key_backup: String,
    /// Whether the API key text input is in edit mode.
    pub api_key_editing: bool,
    /// Result of the last connection test.
    pub connection_status: LlmConnectionStatus,
}

impl std::fmt::Debug for LlmSetupState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmSetupState")
            .field("active_section", &self.active_section)
            .field("selected_provider_idx", &self.selected_provider_idx)
            .field("selected_model_idx", &self.selected_model_idx)
            .field("api_key_input", &if self.api_key_input.is_empty() { "(empty)" } else { "[REDACTED]" })
            .field("api_key_backup", &if self.api_key_backup.is_empty() { "(empty)" } else { "[REDACTED]" })
            .field("api_key_editing", &self.api_key_editing)
            .field("connection_status", &self.connection_status)
            .finish()
    }
}

impl Default for LlmSetupState {
    fn default() -> Self {
        LlmSetupState {
            active_section: LlmSetupSection::Provider,
            selected_provider_idx: 0,
            selected_model_idx: 0,
            api_key_input: String::new(),
            api_key_backup: String::new(),
            api_key_editing: false,
            connection_status: LlmConnectionStatus::Untested,
        }
    }
}

impl LlmSetupState {
    /// The ordered list of providers for selection.
    pub const PROVIDERS: &[LlmProvider] = &[
        LlmProvider::Anthropic,
        LlmProvider::Google,
        LlmProvider::OpenAI,
    ];

    /// Return the currently selected provider.
    pub fn selected_provider(&self) -> &LlmProvider {
        &Self::PROVIDERS[self.selected_provider_idx]
    }

    /// Return the models available for the currently selected provider.
    pub fn available_models(&self) -> Vec<&'static ModelOption> {
        models_for_provider(self.selected_provider())
    }

    /// Return the currently selected model, if any.
    pub fn selected_model(&self) -> Option<&'static ModelOption> {
        let models = self.available_models();
        models.get(self.selected_model_idx).copied()
    }

    /// Change the selected provider by index. Resets model selection to 0
    /// and clears connection test status.
    pub fn set_provider_idx(&mut self, idx: usize) {
        if idx < Self::PROVIDERS.len() && idx != self.selected_provider_idx {
            self.selected_provider_idx = idx;
            self.selected_model_idx = 0;
            self.connection_status = LlmConnectionStatus::Untested;
        }
    }

    /// Move provider selection up.
    pub fn provider_up(&mut self) {
        if self.selected_provider_idx > 0 {
            self.set_provider_idx(self.selected_provider_idx - 1);
        }
    }

    /// Move provider selection down.
    pub fn provider_down(&mut self) {
        if self.selected_provider_idx + 1 < Self::PROVIDERS.len() {
            self.set_provider_idx(self.selected_provider_idx + 1);
        }
    }

    /// Move model selection up.
    pub fn model_up(&mut self) {
        if self.selected_model_idx > 0 {
            self.selected_model_idx -= 1;
        }
    }

    /// Move model selection down.
    pub fn model_down(&mut self) {
        let count = self.available_models().len();
        if self.selected_model_idx + 1 < count {
            self.selected_model_idx += 1;
        }
    }

    /// Return the API key display text: masked when not editing, raw when editing.
    pub fn api_key_display(&self) -> String {
        if self.api_key_input.is_empty() {
            return String::new();
        }
        if self.api_key_editing {
            self.api_key_input.clone()
        } else {
            // Show first 7 chars, then mask the rest
            let visible = self.api_key_input.chars().take(7).collect::<String>();
            let mask_len = self.api_key_input.chars().count().saturating_sub(7);
            format!("{}{}", visible, "*".repeat(mask_len))
        }
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

/// Render the LLM setup screen into the given area.
pub fn render(frame: &mut Frame, area: Rect, state: &LlmSetupState) {
    // Outer block with title
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Line::from(vec![
            Span::styled(
                " Configure Your AI Assistant ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .title_alignment(Alignment::Center);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Vertical layout: provider, model, api key, test button, help bar
    let sections = Layout::vertical([
        Constraint::Length(1),  // top padding
        Constraint::Length(2),  // "Provider:" label + spacing
        Constraint::Length(LlmSetupState::PROVIDERS.len() as u16 + 1), // provider list + spacing
        Constraint::Length(2),  // "Model:" label + spacing
        Constraint::Length(SUPPORTED_MODELS.len().min(4) as u16 + 1), // model list + spacing (max 4 visible)
        Constraint::Length(2),  // "API Key:" label + spacing
        Constraint::Length(2),  // api key input + spacing
        Constraint::Length(2),  // test button + spacing
        Constraint::Length(2),  // connection status
        Constraint::Min(0),    // flexible space
        Constraint::Length(1),  // help bar
    ])
    .split(inner);

    // Horizontal centering: add margin on both sides
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

    // --- Provider section ---
    let provider_active = state.active_section == LlmSetupSection::Provider;
    let provider_label_style = if provider_active {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let provider_label = Paragraph::new(Line::from(vec![
        Span::styled("Provider:", provider_label_style),
    ]));
    frame.render_widget(provider_label, content_rect(sections[1]));

    // Provider list
    let provider_area = content_rect(sections[2]);
    for (i, provider) in LlmSetupState::PROVIDERS.iter().enumerate() {
        let is_selected = i == state.selected_provider_idx;
        let prefix = if is_selected { "> " } else { "  " };
        let style = if is_selected && provider_active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if is_selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        let line = Paragraph::new(Line::from(vec![
            Span::styled(format!("{}{}", prefix, provider.display_name()), style),
        ]));
        let row_rect = Rect {
            x: provider_area.x,
            y: provider_area.y + i as u16,
            width: provider_area.width,
            height: 1,
        };
        frame.render_widget(line, row_rect);
    }

    // --- Model section ---
    let model_active = state.active_section == LlmSetupSection::Model;
    let model_label_style = if model_active {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let model_label = Paragraph::new(Line::from(vec![
        Span::styled("Model:", model_label_style),
    ]));
    frame.render_widget(model_label, content_rect(sections[3]));

    // Model list
    let models = state.available_models();
    let model_area = content_rect(sections[4]);
    for (i, model) in models.iter().enumerate() {
        let is_selected = i == state.selected_model_idx;
        let prefix = if is_selected { "> " } else { "  " };
        let style = if is_selected && model_active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if is_selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };

        let tier_style = Style::default().fg(Color::DarkGray);
        let line = Paragraph::new(Line::from(vec![
            Span::styled(format!("{}{}", prefix, model.display_name), style),
            Span::styled(format!("  {}", model.tier), tier_style),
        ]));
        let row_rect = Rect {
            x: model_area.x,
            y: model_area.y + i as u16,
            width: model_area.width,
            height: 1,
        };
        if row_rect.y < model_area.y + model_area.height {
            frame.render_widget(line, row_rect);
        }
    }

    // --- API Key section ---
    let key_active = state.active_section == LlmSetupSection::ApiKey;
    let key_label_style = if key_active {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let key_label = Paragraph::new(Line::from(vec![
        Span::styled("API Key:", key_label_style),
    ]));
    frame.render_widget(key_label, content_rect(sections[5]));

    // API key input
    let key_area = content_rect(sections[6]);
    let display_text = state.api_key_display();
    let is_empty = display_text.is_empty();
    let key_text = if is_empty {
        if state.api_key_editing {
            String::new()
        } else {
            "(press Enter to input)".to_string()
        }
    } else {
        display_text
    };

    let key_style = if state.api_key_editing {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else if is_empty {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Gray)
    };

    let mut key_spans = vec![
        Span::styled("  ", Style::default()),
        Span::styled(format!("[{}]", key_text), key_style),
    ];

    if state.api_key_editing {
        key_spans.push(Span::styled("|", Style::default().fg(Color::Cyan)));
    }

    let key_para = Paragraph::new(Line::from(key_spans));
    frame.render_widget(key_para, key_area);

    // --- Test Connection button ---
    let test_active = state.active_section == LlmSetupSection::TestButton;
    let test_style = if test_active {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    let test_button = Paragraph::new(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled("[ Test Connection ]", test_style),
    ]));
    frame.render_widget(test_button, content_rect(sections[7]));

    // --- Connection status ---
    let status_area = content_rect(sections[8]);
    let status_line = match &state.connection_status {
        LlmConnectionStatus::Untested => Line::from(vec![
            Span::styled("  Status: ", Style::default().fg(Color::Gray)),
            Span::styled("Not tested", Style::default().fg(Color::DarkGray)),
        ]),
        LlmConnectionStatus::Testing => Line::from(vec![
            Span::styled("  Status: ", Style::default().fg(Color::Gray)),
            Span::styled("Testing...", Style::default().fg(Color::Yellow)),
        ]),
        LlmConnectionStatus::Success(msg) => Line::from(vec![
            Span::styled("  Status: ", Style::default().fg(Color::Gray)),
            Span::styled("* ", Style::default().fg(Color::Green)),
            Span::styled(msg.as_str(), Style::default().fg(Color::Green)),
        ]),
        LlmConnectionStatus::Failed(msg) => Line::from(vec![
            Span::styled("  Status: ", Style::default().fg(Color::Gray)),
            Span::styled("x ", Style::default().fg(Color::Red)),
            Span::styled(msg.as_str(), Style::default().fg(Color::Red)),
        ]),
    };
    frame.render_widget(Paragraph::new(status_line), status_area);

    // --- Help bar ---
    let help_area = content_rect(sections[10]);
    let help_spans = if state.api_key_editing {
        vec![
            Span::styled("Type key", Style::default().fg(Color::Gray)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled("Enter:confirm", Style::default().fg(Color::Gray)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled("Esc:cancel", Style::default().fg(Color::Gray)),
        ]
    } else {
        vec![
            Span::styled("^|v:select", Style::default().fg(Color::Gray)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled("Tab:section", Style::default().fg(Color::Gray)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled("Enter:confirm", Style::default().fg(Color::Gray)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled("n:next", Style::default().fg(Color::Gray)),
            Span::styled(" | ", Style::default().fg(Color::DarkGray)),
            Span::styled("Esc:back", Style::default().fg(Color::Gray)),
        ]
    };
    frame.render_widget(
        Paragraph::new(Line::from(help_spans)).alignment(Alignment::Center),
        help_area,
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_selects_anthropic() {
        let state = LlmSetupState::default();
        assert_eq!(*state.selected_provider(), LlmProvider::Anthropic);
        assert_eq!(state.selected_provider_idx, 0);
        assert_eq!(state.selected_model_idx, 0);
        assert!(!state.api_key_editing);
        assert_eq!(state.connection_status, LlmConnectionStatus::Untested);
    }

    #[test]
    fn provider_down_up() {
        let mut state = LlmSetupState::default();
        state.provider_down();
        assert_eq!(*state.selected_provider(), LlmProvider::Google);
        assert_eq!(state.selected_model_idx, 0); // reset on change

        state.provider_down();
        assert_eq!(*state.selected_provider(), LlmProvider::OpenAI);

        // At bottom, stays at bottom
        state.provider_down();
        assert_eq!(*state.selected_provider(), LlmProvider::OpenAI);

        state.provider_up();
        assert_eq!(*state.selected_provider(), LlmProvider::Google);

        state.provider_up();
        assert_eq!(*state.selected_provider(), LlmProvider::Anthropic);

        // At top, stays at top
        state.provider_up();
        assert_eq!(*state.selected_provider(), LlmProvider::Anthropic);
    }

    #[test]
    fn model_selection_resets_on_provider_change() {
        let mut state = LlmSetupState::default();
        state.model_down(); // select second model
        assert_eq!(state.selected_model_idx, 1);

        state.provider_down(); // change provider
        assert_eq!(state.selected_model_idx, 0); // reset
    }

    #[test]
    fn model_up_down() {
        let mut state = LlmSetupState::default();
        let model_count = state.available_models().len();
        assert!(model_count >= 2);

        state.model_down();
        assert_eq!(state.selected_model_idx, 1);

        // Try to go past end
        for _ in 0..10 {
            state.model_down();
        }
        assert_eq!(state.selected_model_idx, model_count - 1);

        state.model_up();
        assert_eq!(state.selected_model_idx, model_count - 2);

        // Back to top
        for _ in 0..10 {
            state.model_up();
        }
        assert_eq!(state.selected_model_idx, 0);
    }

    #[test]
    fn api_key_display_masked_when_not_editing() {
        let mut state = LlmSetupState::default();
        state.api_key_input = "sk-ant-api03-abcdef123456789".to_string();
        state.api_key_editing = false;

        let display = state.api_key_display();
        assert!(display.starts_with("sk-ant-"));
        assert!(display.contains('*'));
        assert_eq!(display.len(), state.api_key_input.len());
    }

    #[test]
    fn api_key_display_visible_when_editing() {
        let mut state = LlmSetupState::default();
        state.api_key_input = "sk-ant-api03-abcdef123456789".to_string();
        state.api_key_editing = true;

        let display = state.api_key_display();
        assert_eq!(display, state.api_key_input);
    }

    #[test]
    fn api_key_display_empty() {
        let state = LlmSetupState::default();
        assert!(state.api_key_display().is_empty());
    }

    #[test]
    fn section_next_wraps() {
        let s = LlmSetupSection::Provider;
        assert_eq!(s.next(), LlmSetupSection::Model);
        assert_eq!(s.next().next(), LlmSetupSection::ApiKey);
        assert_eq!(s.next().next().next(), LlmSetupSection::TestButton);
        assert_eq!(s.next().next().next().next(), LlmSetupSection::Provider);
    }

    #[test]
    fn section_prev_wraps() {
        let s = LlmSetupSection::Provider;
        assert_eq!(s.prev(), LlmSetupSection::TestButton);
        assert_eq!(s.prev().prev(), LlmSetupSection::ApiKey);
    }

    #[test]
    fn selected_model_returns_correct_model() {
        let state = LlmSetupState::default();
        let model = state.selected_model().unwrap();
        assert_eq!(model.provider, LlmProvider::Anthropic);
    }

    #[test]
    fn connection_status_default() {
        assert_eq!(LlmConnectionStatus::default(), LlmConnectionStatus::Untested);
    }

    #[test]
    fn render_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let state = LlmSetupState::default();
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_with_api_key_editing() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let mut state = LlmSetupState::default();
        state.api_key_editing = true;
        state.api_key_input = "sk-test".to_string();
        state.active_section = LlmSetupSection::ApiKey;
        terminal
            .draw(|frame| render(frame, frame.area(), &state))
            .unwrap();
    }

    #[test]
    fn render_with_connection_status_variants() {
        let backend = ratatui::backend::TestBackend::new(80, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        for status in [
            LlmConnectionStatus::Untested,
            LlmConnectionStatus::Testing,
            LlmConnectionStatus::Success("Connected!".to_string()),
            LlmConnectionStatus::Failed("Invalid API key".to_string()),
        ] {
            let mut state = LlmSetupState::default();
            state.connection_status = status;
            terminal
                .draw(|frame| render(frame, frame.area(), &state))
                .unwrap();
        }
    }
}
