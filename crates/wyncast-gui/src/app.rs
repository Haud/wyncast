use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::keyboard::key::Named;
use iced::{Element, Subscription, Task};
use tokio::sync::mpsc;
use twui::{Toaster, ToastType};
use wyncast_app::protocol::{
    AppMode, ConnectionStatus, LlmStreamUpdate, TabId, UiUpdate, UserCommand,
};

use crate::bridge;
use crate::focus::FocusTarget;
use crate::message::Message;
use crate::modals::ModalKind;
use crate::persistence::{self, LayoutConfig};
use crate::screens::draft::{Direction, DraftEffect, DraftMessage, DraftScreen};
use crate::screens::draft::sidebar::{SidebarMessage};
use crate::screens::draft::sidebar::nomination_plan::PlanMessage;
use crate::screens::draft::sidebar::roster::RosterMessage;
use crate::screens::draft::tabs::available::AvailableMessage;
use crate::screens::matchup::MatchupScreen;
use crate::screens::onboarding::{OnboardingMessage, OnboardingScreen};
use crate::screens::settings::{SettingsMessage, SettingsScreen};
use crate::widgets::SplitPaneState;
use crate::widgets::keyboard_help_overlay;

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
    /// Matchup screen state.
    matchup: MatchupScreen,
    /// Onboarding wizard state.
    onboarding: OnboardingScreen,
    /// Settings screen state.
    settings: SettingsScreen,
    /// Current window width — used to toggle sidebar visibility.
    window_width: f32,
    /// Current window height — for persistence.
    window_height: f32,
    /// Last-known window position — for persistence.
    window_x: Option<i32>,
    window_y: Option<i32>,
    /// Toast notification manager.
    toaster: Toaster,
    /// Whether the keyboard help overlay is visible.
    help_open: bool,
    /// Draggable main/sidebar split state.
    pane_state: SplitPaneState,
}

impl App {
    pub fn new(
        ui_rx: mpsc::Receiver<UiUpdate>,
        cmd_tx: mpsc::Sender<UserCommand>,
        initial_mode: AppMode,
        layout: LayoutConfig,
    ) -> Self {
        Self {
            ui_rx: Arc::new(Mutex::new(Some(ui_rx))),
            cmd_tx,
            app_mode: initial_mode,
            focus: FocusTarget::default(),
            connection_status: ConnectionStatus::Disconnected,
            draft: DraftScreen::new(),
            matchup: MatchupScreen::new(),
            onboarding: OnboardingScreen::new(),
            settings: SettingsScreen::new(),
            window_width: layout.window_width,
            window_height: layout.window_height,
            window_x: layout.window_x,
            window_y: layout.window_y,
            toaster: Toaster::new(),
            help_open: false,
            pane_state: SplitPaneState::new(layout.pane_ratio),
        }
    }

    fn send_command(&self, cmd: UserCommand) {
        if self.cmd_tx.try_send(cmd).is_err() {
            tracing::warn!("cmd_tx full or closed, command dropped");
        }
    }

    fn layout_config(&self) -> LayoutConfig {
        LayoutConfig {
            pane_ratio: self.pane_state.ratio(),
            window_width: self.window_width,
            window_height: self.window_height,
            window_x: self.window_x,
            window_y: self.window_y,
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
                UiUpdate::ModeChanged(mode) => {
                    let entering_settings = matches!(mode, AppMode::Settings(_))
                        && !matches!(app.app_mode, AppMode::Settings(_));
                    app.app_mode = mode.clone();
                    if entering_settings {
                        app.settings.reset_dirty();
                    }
                    if let AppMode::Settings(section) = mode {
                        app.settings.active_section = *section;
                    }
                }
                UiUpdate::ConnectionStatus(status) => {
                    let old = app.connection_status;
                    app.connection_status = *status;
                    if old != *status {
                        match status {
                            ConnectionStatus::Connected => {
                                app.toaster.show(
                                    ToastType::Success,
                                    "Connected",
                                    "ESPN draft extension connected",
                                );
                            }
                            ConnectionStatus::Disconnected => {
                                app.toaster.show(
                                    ToastType::Warning,
                                    "Disconnected",
                                    "Waiting for ESPN draft extension…",
                                );
                            }
                        }
                    }
                }
                UiUpdate::LlmUpdate { update: LlmStreamUpdate::Error(err), .. } => {
                    app.toaster.show(
                        ToastType::Error,
                        "Analysis error",
                        err.clone(),
                    );
                }
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
                UiUpdate::MatchupSnapshot(snapshot) => {
                    app.matchup.apply_snapshot(snapshot);
                    Task::none()
                }
                UiUpdate::OnboardingUpdate(update) => {
                    if matches!(app.app_mode, AppMode::Settings(_)) {
                        app.settings.apply_update(&update);
                    } else {
                        app.onboarding.apply_update(update);
                    }
                    Task::none()
                }
                _ => Task::none(),
            }
        }
        Message::KeyPressed(key, mods) => {
            // Help overlay swallows everything except Esc.
            if app.help_open {
                if matches!(key, iced::keyboard::Key::Named(Named::Escape)) {
                    app.help_open = false;
                }
                return Task::none();
            }

            match &app.app_mode {
                AppMode::Onboarding(_) => handle_onboarding_key(app, &key),
                AppMode::Settings(_) => handle_settings_key(app, &key, mods.shift()),
                AppMode::Matchup => handle_matchup_key(app, &key, mods.shift()),
                _ => {
                    if app.connection_status == ConnectionStatus::Disconnected {
                        return handle_disconnected_key(app, &key);
                    }
                    if app.draft.has_modal() {
                        return handle_modal_key(app, &key);
                    }
                    handle_global_key(app, &key, mods.shift())
                }
            }
        }
        Message::WindowResized(size) => {
            app.window_width = size.width;
            app.window_height = size.height;
            app.matchup.show_sidebar = size.width >= 1100.0;
            Task::none()
        }
        Message::WindowMoved { x, y } => {
            app.window_x = Some(x);
            app.window_y = Some(y);
            Task::none()
        }
        Message::WindowClosed => {
            persistence::save(&app.layout_config());
            Task::none()
        }
        Message::ToastDismissed(id) => {
            app.toaster.dismiss(id);
            Task::none()
        }
        Message::HelpToggled => {
            app.help_open = !app.help_open;
            Task::none()
        }
        Message::SpinnerTick => Task::none(),
        Message::Draft(draft_msg) => dispatch_draft(app, draft_msg),
        Message::Matchup(matchup_msg) => {
            app.matchup.update(matchup_msg).map(Message::Matchup)
        }
        Message::Onboarding(msg) => dispatch_onboarding(app, msg),
        Message::Settings(msg) => dispatch_settings(app, msg),
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
            DraftEffect::PaneResized(event) => {
                app.pane_state.handle_resize(event);
                persistence::save(&app.layout_config());
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

fn dispatch_onboarding(app: &mut App, msg: OnboardingMessage) -> Task<Message> {
    let (task, cmds) = app.onboarding.update(msg);
    for cmd in cmds {
        app.send_command(cmd);
    }
    task.map(Message::Onboarding)
}

fn dispatch_settings(app: &mut App, msg: SettingsMessage) -> Task<Message> {
    let (task, cmds) = app.settings.update(msg);
    for cmd in cmds {
        app.send_command(cmd);
    }
    task.map(Message::Settings)
}

fn handle_onboarding_key(app: &mut App, key: &iced::keyboard::Key) -> Task<Message> {
    match key {
        iced::keyboard::Key::Character(c) if c.as_str() == "?" => {
            app.help_open = true;
            Task::none()
        }
        iced::keyboard::Key::Named(Named::Enter) => {
            dispatch_onboarding(app, OnboardingMessage::Next)
        }
        iced::keyboard::Key::Named(Named::Escape) => {
            dispatch_onboarding(app, OnboardingMessage::Back)
        }
        _ => Task::none(),
    }
}

fn handle_settings_key(app: &mut App, key: &iced::keyboard::Key, shift: bool) -> Task<Message> {
    match key {
        iced::keyboard::Key::Character(c) if c.as_str() == "?" => {
            app.help_open = true;
            Task::none()
        }
        iced::keyboard::Key::Named(Named::Escape) => {
            dispatch_settings(app, SettingsMessage::CancelRequested)
        }
        iced::keyboard::Key::Character(c) if c.as_str() == "," => {
            dispatch_settings(app, SettingsMessage::CancelRequested)
        }
        iced::keyboard::Key::Named(Named::Tab) => {
            if app.settings.discard_modal_open {
                return Task::none();
            }
            use wyncast_app::protocol::SettingsSection;
            let next_section = if shift {
                match app.settings.active_section {
                    SettingsSection::LlmConfig => SettingsSection::StrategyConfig,
                    SettingsSection::StrategyConfig => SettingsSection::LlmConfig,
                }
            } else {
                match app.settings.active_section {
                    SettingsSection::LlmConfig => SettingsSection::StrategyConfig,
                    SettingsSection::StrategyConfig => SettingsSection::LlmConfig,
                }
            };
            dispatch_settings(app, SettingsMessage::SectionSelected(next_section))
        }
        iced::keyboard::Key::Named(Named::Enter)
            if app.settings.discard_modal_open =>
        {
            dispatch_settings(app, SettingsMessage::DiscardConfirmed)
        }
        _ => Task::none(),
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

fn handle_disconnected_key(app: &mut App, key: &iced::keyboard::Key) -> Task<Message> {
    if let iced::keyboard::Key::Character(c) = key {
        if c.as_str() == "r" {
            return dispatch_draft(app, DraftMessage::RetryConnection);
        }
    }
    Task::none()
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
            "?" => {
                app.help_open = !app.help_open;
                Task::none()
            }
            "q" => dispatch_draft(app, DraftMessage::QuitRequested),
            "," => {
                app.settings.reset_dirty();
                app.send_command(UserCommand::OpenSettings);
                Task::none()
            }
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

fn handle_matchup_key(app: &mut App, key: &iced::keyboard::Key, shift: bool) -> Task<Message> {
    use crate::screens::matchup::MatchupMessage;
    match key {
        iced::keyboard::Key::Character(c) => match c.as_str() {
            "?" => {
                app.help_open = !app.help_open;
                Task::none()
            }
            "1" => app.matchup.update(MatchupMessage::TabSelected(
                crate::screens::matchup::tabs::MatchupTab::DailyStats,
            )).map(Message::Matchup),
            "2" => app.matchup.update(MatchupMessage::TabSelected(
                crate::screens::matchup::tabs::MatchupTab::Analytics,
            )).map(Message::Matchup),
            "3" => app.matchup.update(MatchupMessage::TabSelected(
                crate::screens::matchup::tabs::MatchupTab::HomeRoster,
            )).map(Message::Matchup),
            "4" => app.matchup.update(MatchupMessage::TabSelected(
                crate::screens::matchup::tabs::MatchupTab::AwayRoster,
            )).map(Message::Matchup),
            "j" => app.matchup.update(MatchupMessage::ScrollRequested(
                wyncast_app::protocol::ScrollDirection::Down,
            )).map(Message::Matchup),
            "k" => app.matchup.update(MatchupMessage::ScrollRequested(
                wyncast_app::protocol::ScrollDirection::Up,
            )).map(Message::Matchup),
            "q" => dispatch_draft(app, DraftMessage::QuitRequested),
            _ => Task::none(),
        },
        iced::keyboard::Key::Named(Named::Tab) => {
            let msg = if shift {
                MatchupMessage::FocusToggledBack
            } else {
                MatchupMessage::FocusToggled
            };
            app.matchup.update(msg).map(Message::Matchup)
        }
        iced::keyboard::Key::Named(Named::ArrowLeft) => {
            app.matchup.update(MatchupMessage::PreviousDay).map(Message::Matchup)
        }
        iced::keyboard::Key::Named(Named::ArrowRight) => {
            app.matchup.update(MatchupMessage::NextDay).map(Message::Matchup)
        }
        iced::keyboard::Key::Named(Named::ArrowUp) => {
            app.matchup.update(MatchupMessage::ScrollRequested(
                wyncast_app::protocol::ScrollDirection::Up,
            )).map(Message::Matchup)
        }
        iced::keyboard::Key::Named(Named::ArrowDown) => {
            app.matchup.update(MatchupMessage::ScrollRequested(
                wyncast_app::protocol::ScrollDirection::Down,
            )).map(Message::Matchup)
        }
        iced::keyboard::Key::Named(Named::PageUp) => {
            app.matchup.update(MatchupMessage::ScrollRequested(
                wyncast_app::protocol::ScrollDirection::PageUp,
            )).map(Message::Matchup)
        }
        iced::keyboard::Key::Named(Named::PageDown) => {
            app.matchup.update(MatchupMessage::ScrollRequested(
                wyncast_app::protocol::ScrollDirection::PageDown,
            )).map(Message::Matchup)
        }
        _ => Task::none(),
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

pub fn view(app: &App) -> Element<'_, Message> {
    let screen_elem = match &app.app_mode {
        AppMode::Draft => {
            let draft_elem = crate::screens::draft::view(
                &app.draft,
                app.focus,
                app.connection_status,
                &app.pane_state,
            );
            draft_elem.map(Message::Draft)
        }
        AppMode::Onboarding(step) => {
            crate::screens::onboarding::view(&app.onboarding, step).map(Message::Onboarding)
        }
        AppMode::Matchup => {
            crate::screens::matchup::view(&app.matchup).map(Message::Matchup)
        }
        AppMode::Settings(_section) => {
            crate::screens::settings::view(&app.settings).map(Message::Settings)
        }
    };

    // Layer toasts and help overlay on top of the screen content.
    let with_toasts = if let Some(toast_elem) = twui::ToastContainer::view(
        &app.toaster,
        Message::ToastDismissed,
        None::<fn(u64) -> Message>,
    ) {
        iced::widget::stack![screen_elem, toast_elem].into()
    } else {
        screen_elem
    };

    if app.help_open {
        let sections = match &app.app_mode {
            AppMode::Draft => keyboard_help_overlay::draft_sections(),
            AppMode::Matchup => keyboard_help_overlay::matchup_sections(),
            AppMode::Settings(_) => keyboard_help_overlay::settings_sections(),
            AppMode::Onboarding(_) => keyboard_help_overlay::onboarding_sections(),
        };
        let overlay = keyboard_help_overlay::keyboard_help_overlay(
            sections,
            Message::HelpToggled,
        );
        iced::widget::stack![with_toasts, overlay].into()
    } else {
        with_toasts
    }
}

// ---------------------------------------------------------------------------
// Subscription
// ---------------------------------------------------------------------------

pub fn subscription(app: &App) -> Subscription<Message> {
    let needs_tick = app.connection_status == ConnectionStatus::Disconnected
        || app.toaster.has_active_animations();

    let mut subs = vec![
        bridge::ui_subscription_from_arc(app.ui_rx.clone()),
        iced::keyboard::listen().map(|event| match event {
            iced::keyboard::Event::KeyPressed { key, modifiers, .. } => {
                Message::KeyPressed(key, modifiers)
            }
            _ => Message::NoOp,
        }),
        iced::event::listen_with(|event, _status, _id| match event {
            iced::Event::Window(iced::window::Event::Resized(size)) => {
                Some(Message::WindowResized(size))
            }
            iced::Event::Window(iced::window::Event::Moved(point)) => {
                Some(Message::WindowMoved { x: point.x as i32, y: point.y as i32 })
            }
            iced::Event::Window(iced::window::Event::CloseRequested) => {
                Some(Message::WindowClosed)
            }
            _ => None,
        }),
    ];

    if needs_tick {
        subs.push(
            iced::time::every(Duration::from_millis(50)).map(|_| Message::SpinnerTick),
        );
    }

    Subscription::batch(subs)
}
