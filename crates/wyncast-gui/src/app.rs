use std::sync::{Arc, Mutex};

use iced::widget::{column, container, scrollable, text};
use iced::{Element, Length, Subscription, Task};
use tokio::sync::mpsc;
use wyncast_app::protocol::{AppMode, UiUpdate, UserCommand};

use crate::bridge;
use crate::focus::FocusTarget;
use crate::message::Message;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct App {
    /// Backend → GUI channel. Wrapped in Arc<Mutex<Option>> so the receiver
    /// can be moved into the subscription stream (FnOnce) while subscription()
    /// takes &self (Fn). After first render the Option is None; the stream
    /// keeps running by hash identity.
    ui_rx: Arc<Mutex<Option<mpsc::Receiver<UiUpdate>>>>,
    /// GUI → backend channel. Used via send_command().
    #[allow(dead_code)]
    cmd_tx: mpsc::Sender<UserCommand>,
    /// Current app mode (displayed in debug view; drives screen routing from 3.1).
    initial_mode: AppMode,
    /// Keyboard focus target (displayed in debug view; drives focus ring from 3.1).
    focus: FocusTarget,
    /// Debug log: one discriminant string per UiUpdate received.
    updates: Vec<String>,
}

impl App {
    pub fn new(
        ui_rx: mpsc::Receiver<UiUpdate>,
        cmd_tx: mpsc::Sender<UserCommand>,
        initial_mode: AppMode,
    ) -> Self {
        Self {
            ui_rx: Arc::new(Mutex::new(Some(ui_rx))),
            cmd_tx,
            initial_mode,
            focus: FocusTarget::default(),
            updates: Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub fn send_command(&self, cmd: UserCommand) {
        if self.cmd_tx.try_send(cmd).is_err() {
            tracing::warn!("cmd_tx full or closed, command dropped");
        }
    }
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

pub fn update(app: &mut App, msg: Message) -> Task<Message> {
    match msg {
        Message::UiUpdate(update) => {
            app.updates.push(variant_name(&update));
            Task::none()
        }
        Message::KeyPressed(key, _mods) => {
            if matches!(
                key,
                iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape)
            ) {
                return iced::exit();
            }
            Task::none()
        }
        Message::NoOp => Task::none(),
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub fn view(app: &App) -> Element<'_, Message> {
    let log_items: Vec<Element<Message>> = if app.updates.is_empty() {
        vec![text("Waiting for backend updates…").into()]
    } else {
        app.updates
            .iter()
            .enumerate()
            .map(|(i, s)| text(format!("{i}: {s}")).into())
            .collect()
    };

    let log = scrollable(column(log_items).spacing(2)).height(Length::Fill);

    container(
        column([
            text("Wyncast").size(24).into(),
            text(format!(
                "Mode: {:?} | Focus: {:?} | {} update(s) (Esc to quit)",
                app.initial_mode,
                app.focus,
                app.updates.len()
            ))
            .into(),
            log.into(),
        ])
        .spacing(8)
        .padding(16),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

// ---------------------------------------------------------------------------
// Subscription
// ---------------------------------------------------------------------------

pub fn subscription(app: &App) -> Subscription<Message> {
    Subscription::batch([
        bridge::ui_subscription_from_arc(app.ui_rx.clone()),
        iced::keyboard::listen().map(|event| match event {
            iced::keyboard::Event::KeyPressed { key, modifiers, .. } => {
                Message::KeyPressed(key, modifiers)
            }
            _ => Message::NoOp,
        }),
    ])
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn variant_name(update: &UiUpdate) -> String {
    match update {
        UiUpdate::StateSnapshot(_) => "StateSnapshot",
        UiUpdate::LlmUpdate { .. } => "LlmUpdate",
        UiUpdate::ConnectionStatus(_) => "ConnectionStatus",
        UiUpdate::NominationUpdate { .. } => "NominationUpdate",
        UiUpdate::BidUpdate(_) => "BidUpdate",
        UiUpdate::NominationCleared => "NominationCleared",
        UiUpdate::PlanStarted { .. } => "PlanStarted",
        UiUpdate::OnboardingUpdate(_) => "OnboardingUpdate",
        UiUpdate::ModeChanged(_) => "ModeChanged",
        UiUpdate::MatchupSnapshot(_) => "MatchupSnapshot",
    }
    .to_string()
}
