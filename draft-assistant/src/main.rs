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

mod app;
mod config;
mod db;
mod draft;
mod llm;
mod protocol;
mod tui;
mod valuation;
mod ws_server;

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

    let mut available_players = valuation::compute_initial(&projections, &config)
        .context("failed to compute initial valuations")?;
    info!(
        "Computed valuations for {} players",
        available_players.len()
    );

    // 5. Initialize DraftState
    let teams: Vec<(String, String)> = config
        .league
        .teams
        .iter()
        .map(|(id, name)| (id.clone(), name.clone()))
        .collect();
    let draft_state = draft::state::DraftState::new(
        teams,
        &config.league.my_team.team_id,
        config.league.salary_cap,
        &config.league.roster,
    );

    // Create the application state
    let mut app_state = app::AppState::new(
        config.clone(),
        draft_state,
        available_players,
        projections,
        db,
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

    // 6. Create mpsc channels
    let (ws_tx, ws_rx) = mpsc::channel(256);
    let (llm_tx, llm_rx) = mpsc::channel(256);
    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let (ui_tx, ui_rx) = mpsc::channel(256);

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

    // 9. TUI placeholder: wait for Ctrl+C
    // Real TUI implementation is in Tasks 13/15/17.
    info!("Application ready. Press Ctrl+C to exit.");
    info!("WebSocket server listening on 127.0.0.1:{}", ws_port);

    // Drop unused channel endpoints for clean shutdown
    drop(llm_tx); // No LLM producer yet (Task 14)
    drop(ui_rx); // No TUI consumer yet (Task 13)

    // Wait for Ctrl+C
    match tokio::signal::ctrl_c().await {
        Ok(()) => {
            info!("Ctrl+C received, shutting down...");
        }
        Err(e) => {
            error!("Failed to listen for Ctrl+C: {}", e);
        }
    }

    // 10. Cleanup: send quit command to trigger graceful shutdown
    let _ = cmd_tx.send(protocol::UserCommand::Quit).await;

    // Wait for tasks to finish (with timeout)
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
