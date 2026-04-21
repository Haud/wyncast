// Wyncast GUI entry point.
//
// Startup sequence:
// 1. Initialize tracing (log to file)
// 2. Load config + check onboarding status
// 3. Open database, clear stale draft state
// 4. Load projections, initialize DraftState
// 5. Create mpsc channels
// 6. Create tokio runtime; bind WebSocket server; spawn backend tasks
// 7. Launch Iced with the channel pair (ui_rx, cmd_tx)

mod app;
mod bridge;
mod focus;
mod message;
mod theme;
mod widgets;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use tracing::info;

fn main() -> anyhow::Result<()> {
    // 1. Tracing
    init_tracing()?;
    info!("Wyncast GUI starting up");

    // 2. Config + onboarding check
    let config = wyncast_core::config::load_config().context("failed to load configuration")?;
    info!(
        "Config loaded: league={}, {} teams, ${} salary cap",
        config.league.name, config.league.num_teams, config.league.salary_cap
    );

    let onboarding_manager = wyncast_app::onboarding::OnboardingManager::new(
        wyncast_core::app_dirs::config_dir(),
        wyncast_app::onboarding::RealFileSystem,
    );
    let initial_mode = if onboarding_manager.is_configured(&config.credentials) {
        info!("Onboarding complete, starting in draft mode");
        wyncast_app::protocol::AppMode::Draft
    } else {
        let progress = onboarding_manager.load_progress();
        info!("Onboarding needed (step: {:?})", progress.current_step);
        wyncast_app::protocol::AppMode::Onboarding(progress.current_step)
    };

    // 3. Database
    let db_path = wyncast_core::app_dirs::db_path();
    let db_path_str = db_path.to_str().context("database path contains non-UTF-8 characters")?;
    let db = wyncast_core::db::Database::open(db_path_str).context("failed to open database")?;
    info!("Database opened at {db_path_str}");

    db.clear_all_drafts().context("failed to clear persisted draft state")?;
    let draft_id = {
        let id = wyncast_core::db::Database::generate_draft_id();
        db.set_draft_id(&id)?;
        info!("Starting new draft session: {id}");
        id
    };

    // 4. Projections + DraftState
    info!("Loading projections…");
    let projections = wyncast_baseball::valuation::projections::load_all(&config)
        .context("failed to load projections")?;
    let draft_state =
        wyncast_baseball::draft::state::DraftState::new(config.league.salary_cap, &HashMap::new());

    // 5. Channels
    let (ws_tx, ws_rx) = tokio::sync::mpsc::channel(256);
    let (ws_outbound_tx, ws_outbound_rx) = tokio::sync::mpsc::channel(64);
    let (llm_tx, llm_rx) = tokio::sync::mpsc::channel(256);
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(64);
    let (ui_tx, ui_rx) = tokio::sync::mpsc::channel(256);

    let llm_client = wyncast_llm::client::LlmClient::from_config(&config);

    let app_state = wyncast_app::app::AppState::new(
        config.clone(),
        draft_state,
        Vec::new(),  // available_players deferred until ESPN connection
        projections,
        db,
        draft_id,
        llm_client,
        llm_tx.clone(),
        Some(ws_outbound_tx),
        initial_mode.clone(),
        onboarding_manager,
        None,  // roster_config deferred
    );

    let ws_port = config.ws_port;

    // 6. Tokio runtime + async setup
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;

    let listener =
        rt.block_on(wyncast_core::ws_server::TungsteniteListener::bind(ws_port))
            .with_context(|| format!("failed to bind WebSocket server on port {ws_port}"))?;

    info!("WebSocket server listening on 127.0.0.1:{ws_port}");

    let ws_handle = rt.spawn(async move {
        if let Err(e) = wyncast_core::ws_server::run(listener, ws_tx, ws_outbound_rx).await {
            tracing::error!("WebSocket server error: {e}");
        }
    });
    rt.spawn(async move {
        if let Err(e) = wyncast_app::app::run(ws_rx, llm_rx, cmd_rx, ui_tx, app_state).await {
            tracing::error!("Application loop error: {e}");
        }
    });
    drop(llm_tx);

    // Enter the runtime so Iced's tokio executor can schedule on it.
    let _guard = rt.enter();

    // Boot closure must be `Fn` (not `FnOnce`) for the BootFn trait bound.
    // We wrap the non-Clone items in Arc<Mutex<Option>> and take them once.
    let boot_data = Arc::new(Mutex::new(Some((ui_rx, cmd_tx, initial_mode))));
    let boot = move || {
        let (rx, tx, mode) = boot_data
            .lock()
            .unwrap()
            .take()
            .expect("Iced boot called more than once");
        app::App::new(rx, tx, mode)
    };

    // 7. Launch Iced (blocking until user closes the window or presses Esc)
    iced::application(boot, app::update, app::view)
        .title("Wyncast")
        .subscription(app::subscription)
        .run()
        .context("Iced error")?;

    ws_handle.abort();
    info!("Wyncast GUI shut down");
    Ok(())
}

fn init_tracing() -> anyhow::Result<()> {
    use tracing_subscriber::fmt;
    use tracing_subscriber::EnvFilter;

    let log_dir = wyncast_core::app_dirs::log_dir();
    let log_file = std::fs::File::create(log_dir.join("wyncast-gui.log"))?;

    let subscriber = fmt::Subscriber::builder()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("wyncast_gui=debug,warn")),
        )
        .with_writer(log_file)
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(true)
        .with_line_number(true)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .context("failed to set tracing subscriber")?;

    Ok(())
}
