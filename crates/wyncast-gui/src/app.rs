use std::sync::{Arc, Mutex};

use iced::keyboard::key::Named;
use iced::{Element, Length, Subscription, Task};
use tokio::sync::mpsc;
use wyncast_app::protocol::{
    AppMode, ConnectionStatus, TabId, UiUpdate, UserCommand,
};

use crate::bridge;
use crate::focus::FocusTarget;
use crate::message::Message;
use crate::modals::ModalKind;
use crate::screens::draft::{Direction, DraftEffect, DraftMessage, DraftScreen};
use crate::screens::draft::sidebar::{SidebarMessage};
use crate::screens::draft::sidebar::nomination_plan::PlanMessage;
use crate::screens::draft::sidebar::roster::RosterMessage;
use crate::screens::draft::tabs::available::AvailableMessage;

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
            match update {
                UiUpdate::LlmUpdate { request_id, update: llm_update } => {
                    dispatch_draft(
                        app,
                        DraftMessage::LlmUpdate { request_id, update: llm_update },
                    )
                }
                UiUpdate::PlanStarted { request_id } => {
                    dispatch_draft(app, DraftMessage::PlanStarted { request_id })
                }
                UiUpdate::NominationUpdate { info, analysis_request_id } => {
                    dispatch_draft(
                        app,
                        DraftMessage::Nominated { analysis_request_id, info },
                    )
                }
                UiUpdate::BidUpdate(info) => {
                    dispatch_draft(app, DraftMessage::BidUpdated(info))
                }
                UiUpdate::NominationCleared => {
                    dispatch_draft(app, DraftMessage::NominationCleared)
                }
                UiUpdate::StateSnapshot(snapshot) => {
                    dispatch_draft(app, DraftMessage::StateSnapshot(snapshot))
                }
                _ => Task::none(),
            }
        }
        Message::KeyPressed(key, mods) => {
            if app.draft.has_modal() {
                return handle_modal_key(app, &key);
            }
            handle_global_key(app, &key, mods.shift())
        }
        Message::Draft(draft_msg) => dispatch_draft(app, draft_msg),
        Message::NoOp => Task::none(),
    }
}

fn dispatch_draft(app: &mut App, msg: DraftMessage) -> Task<Message> {
    let (task, effects) = app.draft.update(msg);
    let mut exit = false;
    for effect in effects {
        match effect {
            DraftEffect::SendCommand(cmd) => app.send_command(cmd),
            DraftEffect::CycleFocus(Direction::Forward) => {
                app.focus = app.focus.cycle_forward();
            }
            DraftEffect::CycleFocus(Direction::Reverse) => {
                app.focus = app.focus.cycle_backward();
            }
            DraftEffect::Exit => exit = true,
        }
    }
    let task = task.map(Message::Draft);
    if exit {
        Task::batch([task, iced::exit()])
    } else {
        task
    }
}

fn route_scroll(
    app: &mut App,
    dir: wyncast_app::protocol::ScrollDirection,
) -> Task<Message> {
    use FocusTarget::*;
    let msg = match app.focus {
        MainPanel => DraftMessage::ScrollRequested(dir),
        Roster => DraftMessage::Sidebar(SidebarMessage::Roster(RosterMessage::ScrollBy(dir))),
        Scarcity => DraftMessage::Sidebar(SidebarMessage::ScarcityScrollBy(dir)),
        NominationPlan => {
            DraftMessage::Sidebar(SidebarMessage::Plan(PlanMessage::ScrollBy(dir)))
        }
        None | Budget => return Task::none(),
    };
    dispatch_draft(app, msg)
}

fn handle_modal_key(app: &mut App, key: &iced::keyboard::Key) -> Task<Message> {
    match app.draft.modal_stack.top() {
        Some(ModalKind::PositionFilter) => {
            if matches!(key, iced::keyboard::Key::Named(Named::Escape)) {
                dispatch_draft(
                    app,
                    DraftMessage::Available(AvailableMessage::PositionFilterClosed),
                )
            } else {
                Task::none()
            }
        }
        Some(ModalKind::QuitConfirm) => match key {
            iced::keyboard::Key::Named(Named::Enter) => {
                dispatch_draft(app, DraftMessage::QuitConfirmed)
            }
            iced::keyboard::Key::Named(Named::Escape) => {
                dispatch_draft(app, DraftMessage::QuitCancelled)
            }
            _ => Task::none(),
        },
        None => Task::none(),
    }
}

fn handle_global_key(app: &mut App, key: &iced::keyboard::Key, shift: bool) -> Task<Message> {
    match key {
        iced::keyboard::Key::Character(c) => match c.as_str() {
            "q" => dispatch_draft(app, DraftMessage::QuitRequested),
            "1" => dispatch_draft(app, DraftMessage::TabSelected(TabId::Analysis)),
            "2" => dispatch_draft(app, DraftMessage::TabSelected(TabId::Available)),
            "3" => dispatch_draft(app, DraftMessage::TabSelected(TabId::DraftLog)),
            "4" => dispatch_draft(app, DraftMessage::TabSelected(TabId::Teams)),
            "/" if app.draft.active_tab() == TabId::Available => dispatch_draft(
                app,
                DraftMessage::Available(AvailableMessage::FilterFocused(true)),
            ),
            "p" if app.draft.active_tab() == TabId::Available => dispatch_draft(
                app,
                DraftMessage::Available(AvailableMessage::PositionFilterOpened),
            ),
            "j" => route_scroll(app, wyncast_app::protocol::ScrollDirection::Down),
            "k" => route_scroll(app, wyncast_app::protocol::ScrollDirection::Up),
            _ => Task::none(),
        },
        iced::keyboard::Key::Named(Named::Tab) => {
            if shift {
                dispatch_draft(app, DraftMessage::FocusCycle(Direction::Reverse))
            } else {
                dispatch_draft(app, DraftMessage::FocusCycle(Direction::Forward))
            }
        }
        iced::keyboard::Key::Named(Named::ArrowUp) => {
            route_scroll(app, wyncast_app::protocol::ScrollDirection::Up)
        }
        iced::keyboard::Key::Named(Named::ArrowDown) => {
            route_scroll(app, wyncast_app::protocol::ScrollDirection::Down)
        }
        iced::keyboard::Key::Named(Named::PageUp) => {
            route_scroll(app, wyncast_app::protocol::ScrollDirection::PageUp)
        }
        iced::keyboard::Key::Named(Named::PageDown) => {
            route_scroll(app, wyncast_app::protocol::ScrollDirection::PageDown)
        }
        _ => Task::none(),
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
