// Shared LLM configuration form — used by both onboarding wizard and settings screen.

use iced::{Element, Length};
use twui::{
    BoxStyle, ButtonStyle, ButtonVariant, Colors, DropdownItem, StackAlign, StackGap, StackStyle,
    TextColor, TextSize, TextStyle, TextWeight, button, frame, h_stack, text, text_field, v_stack,
    Dropdown, TextFieldStyle,
};
use wyncast_core::llm::provider::{models_for_provider, LlmProvider};

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum LlmFormMessage {
    ProviderSelected(LlmProvider),
    ModelSelected(String),
    ApiKeyChanged(String),
    ProviderDropdownToggled,
    ModelDropdownToggled,
    TestConnection,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionTestState {
    Idle,
    Testing,
    Success,
    Failed,
}

pub struct LlmFormState {
    pub provider: Option<LlmProvider>,
    pub model_id: Option<String>,
    pub api_key: String,
    /// Masked key from backend sync — indicates a saved key exists.
    pub api_key_mask: Option<String>,
    pub provider_dropdown_open: bool,
    pub model_dropdown_open: bool,
    pub connection_test: ConnectionTestState,
    pub connection_test_message: String,
    // Stored so view() can borrow it for 'a (Dropdown::view takes &'a self)
    dropdown: Dropdown,
}

impl Default for LlmFormState {
    fn default() -> Self {
        Self {
            provider: None,
            model_id: None,
            api_key: String::new(),
            api_key_mask: None,
            provider_dropdown_open: false,
            model_dropdown_open: false,
            connection_test: ConnectionTestState::Idle,
            connection_test_message: String::new(),
            dropdown: Dropdown::new().with_match_width(true),
        }
    }
}

impl LlmFormState {
    pub fn is_valid(&self) -> bool {
        self.provider.is_some()
            && self.model_id.is_some()
            && (!self.api_key.is_empty() || self.api_key_mask.is_some())
    }

    /// Apply a form message to mutate state. Returns true if the provider/model/key
    /// fields changed (i.e., the form is now dirty).
    pub fn apply(&mut self, msg: LlmFormMessage) -> bool {
        match msg {
            LlmFormMessage::ProviderSelected(p) => {
                self.provider = Some(p);
                self.model_id = None;
                self.provider_dropdown_open = false;
                true
            }
            LlmFormMessage::ModelSelected(id) => {
                self.model_id = Some(id);
                self.model_dropdown_open = false;
                true
            }
            LlmFormMessage::ApiKeyChanged(key) => {
                self.api_key = key;
                self.connection_test = ConnectionTestState::Idle;
                true
            }
            LlmFormMessage::ProviderDropdownToggled => {
                self.provider_dropdown_open = !self.provider_dropdown_open;
                self.model_dropdown_open = false;
                false
            }
            LlmFormMessage::ModelDropdownToggled => {
                self.model_dropdown_open = !self.model_dropdown_open;
                self.provider_dropdown_open = false;
                false
            }
            LlmFormMessage::TestConnection => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers (pub so onboarding can reuse)
// ---------------------------------------------------------------------------

pub fn provider_items() -> Vec<DropdownItem> {
    vec![
        DropdownItem {
            key: "anthropic".to_string(),
            label: "Anthropic Claude".to_string(),
            sublabel: None,
        },
        DropdownItem {
            key: "google".to_string(),
            label: "Google Gemini".to_string(),
            sublabel: None,
        },
        DropdownItem {
            key: "openai".to_string(),
            label: "OpenAI".to_string(),
            sublabel: None,
        },
    ]
}

pub fn provider_key(p: &LlmProvider) -> String {
    match p {
        LlmProvider::Anthropic => "anthropic".to_string(),
        LlmProvider::Google => "google".to_string(),
        LlmProvider::OpenAI => "openai".to_string(),
    }
}

pub fn model_items(provider: &LlmProvider) -> Vec<DropdownItem> {
    models_for_provider(provider)
        .into_iter()
        .map(|m| DropdownItem {
            key: m.model_id.to_string(),
            label: m.display_name.to_string(),
            sublabel: Some(m.tier.to_string()),
        })
        .collect()
}

pub fn key_to_provider(key: &str) -> LlmProvider {
    match key {
        "google" => LlmProvider::Google,
        "openai" => LlmProvider::OpenAI,
        _ => LlmProvider::Anthropic,
    }
}

// ---------------------------------------------------------------------------
// View (form fields only — title, provider, model, api key, test row)
// No navigation buttons; those are added by the containing screen.
// ---------------------------------------------------------------------------

pub fn view<'a>(state: &'a LlmFormState) -> Element<'a, LlmFormMessage> {
    // --- Provider dropdown ---
    let provider_label: Element<'a, LlmFormMessage> = text(
        "LLM PROVIDER",
        TextStyle {
            size: TextSize::Xs,
            weight: TextWeight::Semibold,
            color: TextColor::Yellow,
            ..Default::default()
        },
    )
    .into();

    let selected_provider_key = state.provider.as_ref().map(provider_key);
    let provider_dd: Element<'a, LlmFormMessage> = state
        .dropdown
        .view(
            state.provider_dropdown_open,
            provider_items(),
            LlmFormMessage::ProviderDropdownToggled,
            |key| LlmFormMessage::ProviderSelected(key_to_provider(&key)),
            selected_provider_key,
            "Select provider…".to_string(),
        )
        .into();

    let provider_section: Element<'a, LlmFormMessage> = v_stack(
        vec![provider_label, provider_dd],
        StackStyle {
            gap: StackGap::Xs,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into();

    // --- Model dropdown (only when provider is selected) ---
    let model_section: Element<'a, LlmFormMessage> = if let Some(ref provider) = state.provider {
        let model_label: Element<'a, LlmFormMessage> = text(
            "MODEL",
            TextStyle {
                size: TextSize::Xs,
                weight: TextWeight::Semibold,
                color: TextColor::Yellow,
                ..Default::default()
            },
        )
        .into();

        let selected_model_key = state.model_id.clone();
        let model_dd: Element<'a, LlmFormMessage> = state
            .dropdown
            .view(
                state.model_dropdown_open,
                model_items(provider),
                LlmFormMessage::ModelDropdownToggled,
                LlmFormMessage::ModelSelected,
                selected_model_key,
                "Select model…".to_string(),
            )
            .into();

        v_stack(
            vec![model_label, model_dd],
            StackStyle {
                gap: StackGap::Xs,
                width: Length::Fill,
                ..Default::default()
            },
        )
        .into()
    } else {
        iced::widget::Space::new().into()
    };

    // --- API key field ---
    let api_key_placeholder = state.api_key_mask.as_deref().unwrap_or("Enter API key…");

    let api_key_field: Element<'a, LlmFormMessage> = text_field(
        &state.api_key,
        LlmFormMessage::ApiKeyChanged,
        TextFieldStyle::new()
            .label("API KEY")
            .placeholder(api_key_placeholder)
            .secure(true),
    )
    .into();

    // --- Connection test row ---
    let test_feedback: Element<'a, LlmFormMessage> = match state.connection_test {
        ConnectionTestState::Idle => iced::widget::Space::new().into(),
        ConnectionTestState::Testing => text(
            "Testing connection…",
            TextStyle {
                size: TextSize::Sm,
                color: TextColor::Dimmed,
                ..Default::default()
            },
        )
        .into(),
        ConnectionTestState::Success => text(
            format!("✓ {}", state.connection_test_message),
            TextStyle {
                size: TextSize::Sm,
                color: TextColor::Blue,
                ..Default::default()
            },
        )
        .into(),
        ConnectionTestState::Failed => text(
            format!("✗ {}", state.connection_test_message),
            TextStyle {
                size: TextSize::Sm,
                color: TextColor::Error,
                ..Default::default()
            },
        )
        .into(),
    };

    let test_btn: Element<'a, LlmFormMessage> = button(
        text("Test Connection", TextStyle::default()),
        LlmFormMessage::TestConnection,
        ButtonStyle::new().variant(ButtonVariant::Ghost),
    )
    .into();

    let test_row: Element<'a, LlmFormMessage> = h_stack(
        vec![test_btn, test_feedback],
        StackStyle {
            gap: StackGap::Sm,
            align: StackAlign::Center,
            ..Default::default()
        },
    )
    .into();

    // --- Divider ---
    let divider: Element<'a, LlmFormMessage> = frame(
        iced::widget::Space::new().width(Length::Fill),
        BoxStyle {
            background: Some(Colors::BorderSubtle),
            width: Length::Fill,
            height: Length::Fixed(1.0),
            ..Default::default()
        },
    )
    .into();

    v_stack(
        vec![provider_section, model_section, api_key_field, test_row, divider],
        StackStyle {
            gap: StackGap::Md,
            width: Length::Fill,
            ..Default::default()
        },
    )
    .into()
}
