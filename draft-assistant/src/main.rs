// Draft assistant entry point.
//
// Startup sequence:
// 1. Initialize tracing (log to file, not terminal)
// 2. Load config
// 3. Open database, check for crash recovery
// 4. Load projections, compute initial valuations
// 5. Initialize DraftState
// 6. Create mpsc channels
// 7. Spawn WebSocket server task
// 8. Spawn app logic task
// 9. TUI placeholder (wait for Ctrl+C)
// 10. Cleanup on exit

use draft_assistant::app;
use draft_assistant::config;
use draft_assistant::db;
use draft_assistant::draft;
use draft_assistant::llm;
use draft_assistant::tui;
use draft_assistant::valuation;
use draft_assistant::ws_server;

use anyhow::Context;
use tokio::sync::mpsc;
use tracing::{error, info};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Initialize tracing (log to file, not terminal)
    init_tracing()?;
    info!("Draft assistant starting up");

    // 2. Load config
    let config = config::load_config().context("failed to load configuration")?;
    info!(
        "Config loaded: league={}, {} teams, ${} salary cap",
        config.league.name, config.league.num_teams, config.league.salary_cap
    );

    // 3. Open database
    let db = db::Database::open(&config.db_path).context("failed to open database")?;
    info!("Database opened at {}", config.db_path);

    // 4. Load projections and compute initial valuations
    info!("Loading projections...");
    let projections = valuation::projections::load_all(&config)
        .context("failed to load projections")?;
    info!(
        "Loaded {} hitters, {} pitchers",
        projections.hitters.len(),
        projections.pitchers.len()
    );

    let available_players = valuation::compute_initial(&projections, &config)
        .context("failed to compute initial valuations")?;
    info!(
        "Computed valuations for {} players",
        available_players.len()
    );

    // 5. Initialize DraftState (teams populated dynamically from ESPN live data)
    let draft_state = draft::state::DraftState::new(
        config.league.salary_cap,
        &config.league.roster,
    );

    // 6. Create mpsc channels (before AppState so llm_tx can be passed in)
    let (ws_tx, ws_rx) = mpsc::channel(256);
    let (llm_tx, llm_rx) = mpsc::channel(256);
    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let (ui_tx, ui_rx) = mpsc::channel(256);

    // Build the LLM client from config
    let llm_client = llm::client::LlmClient::from_config(&config);
    match &llm_client {
        llm::client::LlmClient::Active(_) => info!("LLM client initialized (API key configured)"),
        llm::client::LlmClient::Disabled => info!("LLM client disabled (no API key)"),
    }

    // Create the application state
    let mut app_state = app::AppState::new(
        config.clone(),
        draft_state,
        available_players,
        projections,
        db,
        llm_client,
        llm_tx.clone(),
    );

    // Check for crash recovery
    match app::recover_from_db(&mut app_state) {
        Ok(true) => info!("Draft state restored from previous session"),
        Ok(false) => info!("Starting fresh draft session"),
        Err(e) => {
            error!("Crash recovery failed: {}", e);
            return Err(e.context("crash recovery failed"));
        }
    }

    // 7. Spawn WebSocket server task
    let ws_port = config.ws_port;
    let ws_handle = tokio::spawn(async move {
        match ws_server::TungsteniteListener::bind(ws_port).await {
            Ok(listener) => {
                if let Err(e) = ws_server::run(listener, ws_tx).await {
                    error!("WebSocket server error: {}", e);
                }
            }
            Err(e) => {
                error!("Failed to bind WebSocket server on port {}: {}", ws_port, e);
            }
        }
    });

    // 8. Spawn app logic task
    let app_handle = tokio::spawn(async move {
        if let Err(e) = app::run(ws_rx, llm_rx, cmd_rx, ui_tx, app_state).await {
            error!("Application loop error: {}", e);
        }
    });

    // 9. Run the TUI event loop (blocking until user quits)
    info!("Application ready. WebSocket server listening on 127.0.0.1:{}", ws_port);

    // Drop the LLM sender clone; AppState holds its own clone for spawning tasks.
    drop(llm_tx);

    // The TUI consumes ui_rx and sends commands through cmd_tx.
    // It blocks until the user presses 'q' or Ctrl+C.
    if let Err(e) = tui::run(ui_rx, cmd_tx).await {
        error!("TUI error: {}", e);
    }

    // 10. Cleanup: wait for app task to finish (with timeout)
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let _ = app_handle.await;
    })
    .await;

    // Abort WebSocket server (it loops forever)
    ws_handle.abort();

    info!("Draft assistant shut down cleanly");
    Ok(())
}

/// Initialize tracing to log to a file (not the terminal, which is used by the TUI).
fn init_tracing() -> anyhow::Result<()> {
    use tracing_subscriber::fmt;
    use tracing_subscriber::EnvFilter;

    let log_dir = std::env::current_dir()?.join("logs");
    std::fs::create_dir_all(&log_dir)?;

    let log_file = std::fs::File::create(log_dir.join("draft-assistant.log"))?;

    let subscriber = fmt::Subscriber::builder()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("draft_assistant=info,warn")),
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
