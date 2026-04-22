use std::sync::{Arc, Mutex};

use iced::keyboard::key::Named;
use iced::widget::operation::{self, AbsoluteOffset};
use iced::{Element, Length, Subscription, Task};
use tokio::sync::mpsc;
use wyncast_app::protocol::{
    AppMode, ConnectionStatus, ScrollDirection, TabId, UiUpdate, UserCommand,
};

use crate::bridge;
use crate::focus::FocusTarget;
use crate::message::Message;
use crate::screens::draft::{Direction, DraftMessage, DraftScreen};
use crate::screens::draft::tabs::analysis::AnalysisMessage;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct App {
    /// Backend → GUI channel.
    ui_rx: Arc<Mutex<Option<mpsc::Receiver<UiUpdate>>>>,
    /// GUI → backend channel.
    cmd_tx: mpsc::Sender<UserCommand>,
    /// Current application mode — drives top-level screen routing.
    app_mode: AppMode,
    /// Keyboard focus target — which panel gets the focus ring.
    focus: FocusTarget,
    /// WebSocket connection state.
    connection_status: ConnectionStatus,
    /// Draft screen state.
    draft: DraftScreen,
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
            app_mode: initial_mode,
            focus: FocusTarget::default(),
            connection_status: ConnectionStatus::Disconnected,
            draft: DraftScreen::new(),
        }
    }

    fn send_command(&self, cmd: UserCommand) {
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
            match &update {
                UiUpdate::ModeChanged(mode) => app.app_mode = mode.clone(),
                UiUpdate::ConnectionStatus(status) => app.connection_status = *status,
                _ => {}
            }
            // Route to draft screen (analysis panel etc.).
            if let Some(snap_id) = app.draft.apply_ui_update(&update) {
                operation::snap_to_end(snap_id)
            } else {
                Task::none()
            }
        }
        Message::KeyPressed(key, mods) => {
            // Priority order: modal → global
            if app.draft.quit_modal_open {
                return handle_modal_key(app, &key);
            }
            handle_global_key(app, &key, mods.shift())
        }
        Message::Draft(draft_msg) => handle_draft_message(app, draft_msg),
        Message::NoOp => Task::none(),
    }
}

fn handle_modal_key(app: &mut App, key: &iced::keyboard::Key) -> Task<Message> {
    match key {
        iced::keyboard::Key::Named(Named::Enter) => {
            handle_draft_message(app, DraftMessage::QuitConfirmed)
        }
        iced::keyboard::Key::Named(Named::Escape) => {
            handle_draft_message(app, DraftMessage::QuitCancelled)
        }
        _ => Task::none(),
    }
}

fn handle_global_key(app: &mut App, key: &iced::keyboard::Key, shift: bool) -> Task<Message> {
    match key {
        iced::keyboard::Key::Character(c) => match c.as_str() {
            "q" => handle_draft_message(app, DraftMessage::QuitRequested),
            "1" => handle_draft_message(app, DraftMessage::TabSelected(TabId::Analysis)),
            "2" => handle_draft_message(app, DraftMessage::TabSelected(TabId::Available)),
            "3" => handle_draft_message(app, DraftMessage::TabSelected(TabId::DraftLog)),
            "4" => handle_draft_message(app, DraftMessage::TabSelected(TabId::Teams)),
            "j" => handle_draft_message(app, DraftMessage::ScrollRequested(ScrollDirection::Down)),
            "k" => handle_draft_message(app, DraftMessage::ScrollRequested(ScrollDirection::Up)),
            _ => Task::none(),
        },
        iced::keyboard::Key::Named(Named::Tab) => {
            if shift {
                handle_draft_message(app, DraftMessage::FocusCycle(Direction::Reverse))
            } else {
                handle_draft_message(app, DraftMessage::FocusCycle(Direction::Forward))
            }
        }
        iced::keyboard::Key::Named(Named::ArrowUp) => {
            handle_draft_message(app, DraftMessage::ScrollRequested(ScrollDirection::Up))
        }
        iced::keyboard::Key::Named(Named::ArrowDown) => {
            handle_draft_message(app, DraftMessage::ScrollRequested(ScrollDirection::Down))
        }
        iced::keyboard::Key::Named(Named::PageUp) => {
            handle_draft_message(app, DraftMessage::ScrollRequested(ScrollDirection::PageUp))
        }
        iced::keyboard::Key::Named(Named::PageDown) => {
            handle_draft_message(app, DraftMessage::ScrollRequested(ScrollDirection::PageDown))
        }
        _ => Task::none(),
    }
}

fn handle_draft_message(app: &mut App, msg: DraftMessage) -> Task<Message> {
    match msg {
        DraftMessage::TabSelected(tab) => {
            app.draft.active_tab = tab;
            app.send_command(UserCommand::SwitchTab(tab));
            Task::none()
        }
        DraftMessage::FocusCycle(Direction::Forward) => {
            app.focus = app.focus.cycle_forward();
            Task::none()
        }
        DraftMessage::FocusCycle(Direction::Reverse) => {
            app.focus = app.focus.cycle_backward();
            Task::none()
        }
        DraftMessage::QuitRequested => {
            app.draft.quit_modal_open = true;
            Task::none()
        }
        DraftMessage::QuitConfirmed => {
            app.send_command(UserCommand::Quit);
            iced::exit()
        }
        DraftMessage::QuitCancelled => {
            app.draft.quit_modal_open = false;
            Task::none()
        }
        DraftMessage::ScrollRequested(dir) => {
            handle_scroll_key(app, dir)
        }
        DraftMessage::Analysis(AnalysisMessage::UserScrolled(rel_y)) => {
            app.draft.analysis.handle_scroll(rel_y);
            Task::none()
        }
    }
}

/// Route a scroll key to the focused panel's scrollable.
fn handle_scroll_key(app: &mut App, dir: ScrollDirection) -> Task<Message> {
    // Only the Analysis panel is implemented in Phase 3.2.
    // When MainPanel is focused and the Analysis tab is active, scroll it.
    if app.focus == FocusTarget::MainPanel && app.draft.active_tab == TabId::Analysis {
        let (dx, dy) = scroll_amount(dir);
        let scroll_id = app.draft.analysis.scroll_id.clone();
        // Disable auto-scroll when user manually scrolls up.
        if dy < 0.0 {
            app.draft.analysis.auto_scroll = false;
        }
        operation::scroll_by(scroll_id, AbsoluteOffset { x: dx, y: dy })
    } else {
        Task::none()
    }
}

fn scroll_amount(dir: ScrollDirection) -> (f32, f32) {
    match dir {
        ScrollDirection::Up => (0.0, -40.0),
        ScrollDirection::Down => (0.0, 40.0),
        ScrollDirection::PageUp => (0.0, -300.0),
        ScrollDirection::PageDown => (0.0, 300.0),
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub fn view(app: &App) -> Element<'_, Message> {
    match &app.app_mode {
        AppMode::Draft => {
            let draft_elem =
                crate::screens::draft::view(&app.draft, app.focus, app.connection_status);
            draft_elem.map(Message::Draft)
        }
        _ => {
            // TODO: unhandled mode — subsequent phases add Onboarding, Settings, Matchup
            iced::widget::container(
                iced::widget::text("TODO: unhandled mode"),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
        }
    }
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
