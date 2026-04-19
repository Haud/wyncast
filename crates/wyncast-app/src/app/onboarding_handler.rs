use std::sync::atomic::Ordering;
use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{info, warn};

use wyncast_core::config::Config;
use wyncast_llm::client::LlmClient;
use wyncast_baseball::llm::prompt;
use crate::onboarding::OnboardingStep;
use crate::protocol::{
    AppMode, OnboardingAction, OnboardingUpdate, UiUpdate,
};
use wyncast_baseball::valuation;
use wyncast_baseball::valuation::scarcity::compute_scarcity;

use super::{AppState, CONNECTION_TEST_FAILED, CONNECTION_TEST_PASSED};

/// Handle a single onboarding action from the TUI.
///
/// Updates `OnboardingProgress` in memory and persists it to disk on
/// step transitions (GoNext/GoBack). Dispatches `UiUpdate` messages
/// back to the TUI so it can reflect changes.
pub(super) async fn handle_onboarding_action(
    state: &mut AppState,
    action: OnboardingAction,
    ui_tx: &mpsc::Sender<UiUpdate>,
) {
    use wyncast_core::llm::provider::LlmProvider;

    match action {
        OnboardingAction::SetProvider(provider) => {
            state.onboarding_progress.llm_provider = Some(provider);
            // Model is reset by the TUI; we just clear it here too for consistency
            state.onboarding_progress.llm_model = None;
        }
        OnboardingAction::SetModel(model_id) => {
            state.onboarding_progress.llm_model = Some(model_id);
        }
        OnboardingAction::SetApiKey(key) => {
            // Persist the API key to credentials.toml for the current provider
            if let Some(ref provider) = state.onboarding_progress.llm_provider {
                save_api_key_for_provider(provider, &key, &mut state.config, &state.onboarding_manager);
            } else {
                // Default to Anthropic if no provider selected yet
                save_api_key_for_provider(&LlmProvider::Anthropic, &key, &mut state.config, &state.onboarding_manager);
            }
            // Auto-trigger connection test so the user doesn't need a second Enter.
            // Box::pin is used for the recursive async call.
            Box::pin(handle_onboarding_action(state, OnboardingAction::TestConnection, ui_tx)).await;
        }
        OnboardingAction::TestConnection => {
            // Spawn an async task to test the API connection
            let provider = state
                .onboarding_progress
                .llm_provider
                .clone()
                .unwrap_or(LlmProvider::Anthropic);
            let model_id = state
                .onboarding_progress
                .llm_model
                .clone()
                .unwrap_or_else(|| "claude-sonnet-4-6".to_string());

            let api_key = get_api_key_for_provider(&provider, &state.config);

            if api_key.is_empty() {
                state.connection_test_result.store(CONNECTION_TEST_FAILED, Ordering::Relaxed);
                let _ = ui_tx
                    .send(UiUpdate::OnboardingUpdate(
                        OnboardingUpdate::ConnectionTestResult {
                            success: false,
                            message: "No API key entered".to_string(),
                        },
                    ))
                    .await;
                return;
            }

            // Increment generation so any in-flight test from a previous
            // trigger is discarded when it completes.
            let generation = state.connection_test_generation.fetch_add(1, Ordering::Relaxed) + 1;

            let tx = ui_tx.clone();
            let tracker = Arc::clone(&state.connection_test_result);
            let gen_tracker = Arc::clone(&state.connection_test_generation);
            tokio::spawn(async move {
                let result = test_api_connection(&provider, &api_key, &model_id).await;
                let success = result.is_ok();
                // Only write if this generation is still current (no newer
                // test or reset has occurred since we started).
                if gen_tracker.load(Ordering::Relaxed) == generation {
                    tracker.store(
                        if success { CONNECTION_TEST_PASSED } else { CONNECTION_TEST_FAILED },
                        Ordering::Relaxed,
                    );
                }
                let _ = tx
                    .send(UiUpdate::OnboardingUpdate(
                        OnboardingUpdate::ConnectionTestResult {
                            success,
                            message: match &result {
                                Ok(msg) => msg.clone(),
                                Err(msg) => msg.clone(),
                            },
                        },
                    ))
                    .await;
            });
        }
        OnboardingAction::GoNext => {
            match state.app_mode {
                AppMode::Onboarding(OnboardingStep::LlmSetup) => {
                    // Block advance if the last connection test explicitly failed.
                    let test_val = state.connection_test_result.load(Ordering::Relaxed);
                    if test_val == CONNECTION_TEST_FAILED {
                        // Test was run and failed — send error back to TUI
                        let _ = ui_tx
                            .send(UiUpdate::OnboardingUpdate(
                                OnboardingUpdate::ConnectionTestResult {
                                    success: false,
                                    message: "Connection test failed — fix the API key or skip to proceed".to_string(),
                                },
                            ))
                            .await;
                        return;
                    }

                    // Save the LLM provider/model to strategy config and persist
                    if let Some(ref provider) = state.onboarding_progress.llm_provider {
                        state.config.strategy.llm.provider = provider.clone();
                    }
                    if let Some(ref model) = state.onboarding_progress.llm_model {
                        state.config.strategy.llm.model = model.clone();
                    }

                    // Reload LLM client so strategy step can use it
                    state.reload_llm_client();

                    // Advance to next step
                    state.onboarding_progress.current_step = OnboardingStep::StrategySetup;
                    if let Err(e) = state
                        .onboarding_manager
                        .save_progress(&state.onboarding_progress)
                    {
                        warn!("Failed to save onboarding progress: {}", e);
                    }

                    state.app_mode =
                        AppMode::Onboarding(OnboardingStep::StrategySetup);
                    let _ = ui_tx
                        .send(UiUpdate::ModeChanged(AppMode::Onboarding(
                            OnboardingStep::StrategySetup,
                        )))
                        .await;
                }
                AppMode::Onboarding(OnboardingStep::StrategySetup) => {
                    // Mark onboarding as complete
                    state.onboarding_progress.current_step = OnboardingStep::Complete;
                    state.onboarding_progress.strategy_configured = true;
                    if let Err(e) = state
                        .onboarding_manager
                        .save_progress(&state.onboarding_progress)
                    {
                        warn!("Failed to save onboarding progress: {}", e);
                    }

                    // Rebuild LLM client with new config
                    state.llm_client = Arc::new(LlmClient::from_config(&state.config));

                    state.app_mode = AppMode::Draft;
                    let _ = ui_tx.send(UiUpdate::ModeChanged(AppMode::Draft)).await;
                    let snapshot = state.build_snapshot();
                    let _ = ui_tx
                        .send(UiUpdate::StateSnapshot(Box::new(snapshot)))
                        .await;
                }
                AppMode::Onboarding(OnboardingStep::Complete) => {
                    // Already complete, go to draft
                    state.app_mode = AppMode::Draft;
                    let _ = ui_tx.send(UiUpdate::ModeChanged(AppMode::Draft)).await;
                    let snapshot = state.build_snapshot();
                    let _ = ui_tx
                        .send(UiUpdate::StateSnapshot(Box::new(snapshot)))
                        .await;
                }
                _ => {
                    // Not in an onboarding mode, ignore
                }
            }
        }
        OnboardingAction::GoBack => {
            match state.app_mode {
                AppMode::Onboarding(OnboardingStep::LlmSetup) => {
                    // Already at first step, no-op
                }
                AppMode::Onboarding(OnboardingStep::StrategySetup) => {
                    state.onboarding_progress.current_step = OnboardingStep::LlmSetup;
                    if let Err(e) = state
                        .onboarding_manager
                        .save_progress(&state.onboarding_progress)
                    {
                        warn!("Failed to save onboarding progress: {}", e);
                    }

                    state.app_mode = AppMode::Onboarding(OnboardingStep::LlmSetup);
                    let _ = ui_tx
                        .send(UiUpdate::OnboardingUpdate(
                            OnboardingUpdate::ProgressSync {
                                provider: state.onboarding_progress.llm_provider.clone(),
                                model: state.onboarding_progress.llm_model.clone(),
                                api_key_mask: None,
                            },
                        ))
                        .await;
                    let _ = ui_tx
                        .send(UiUpdate::ModeChanged(AppMode::Onboarding(
                            OnboardingStep::LlmSetup,
                        )))
                        .await;
                }
                AppMode::Onboarding(OnboardingStep::Complete) => {
                    // Go back to strategy setup
                    state.onboarding_progress.current_step = OnboardingStep::StrategySetup;
                    if let Err(e) = state
                        .onboarding_manager
                        .save_progress(&state.onboarding_progress)
                    {
                        warn!("Failed to save onboarding progress: {}", e);
                    }
                    state.app_mode =
                        AppMode::Onboarding(OnboardingStep::StrategySetup);
                    let _ = ui_tx
                        .send(UiUpdate::ModeChanged(AppMode::Onboarding(
                            OnboardingStep::StrategySetup,
                        )))
                        .await;
                }
                _ => {
                    // Not in an onboarding mode, ignore
                }
            }
        }
        OnboardingAction::Skip => {
            // Save partial progress but keep current_step at the skipped step
            // so re-running the app will resume from this step.
            if let Err(e) = state
                .onboarding_manager
                .save_progress(&state.onboarding_progress)
            {
                warn!("Failed to save onboarding progress on skip: {}", e);
            }

            match state.app_mode {
                AppMode::Onboarding(OnboardingStep::LlmSetup) => {
                    // Skip LlmSetup -> show StrategySetup for this session
                    // but don't advance current_step (stays at LlmSetup)
                    state.app_mode = AppMode::Onboarding(OnboardingStep::StrategySetup);
                    let _ = ui_tx
                        .send(UiUpdate::ModeChanged(AppMode::Onboarding(
                            OnboardingStep::StrategySetup,
                        )))
                        .await;
                }
                AppMode::Onboarding(OnboardingStep::StrategySetup) => {
                    // Skip StrategySetup -> transition to Draft for this session
                    // but don't advance current_step (stays at StrategySetup)
                    state.llm_client = Arc::new(LlmClient::from_config(&state.config));
                    state.app_mode = AppMode::Draft;
                    let _ = ui_tx.send(UiUpdate::ModeChanged(AppMode::Draft)).await;
                    let snapshot = state.build_snapshot();
                    let _ = ui_tx
                        .send(UiUpdate::StateSnapshot(Box::new(snapshot)))
                        .await;
                }
                AppMode::Onboarding(OnboardingStep::Complete) => {
                    // Already complete, go to draft
                    state.app_mode = AppMode::Draft;
                    let _ = ui_tx.send(UiUpdate::ModeChanged(AppMode::Draft)).await;
                    let snapshot = state.build_snapshot();
                    let _ = ui_tx
                        .send(UiUpdate::StateSnapshot(Box::new(snapshot)))
                        .await;
                }
                _ => {
                    // Not in an onboarding mode, ignore
                }
            }
        }
        OnboardingAction::SaveStrategyConfig { hitting_budget_pct, category_weights, strategy_overview } => {
            // Update in-memory config
            state.config.strategy.hitting_budget_fraction = hitting_budget_pct as f64 / 100.0;
            state.config.strategy.weights = category_weights.to_config_weights();
            state.config.strategy.strategy_overview = strategy_overview.clone();

            // Persist to strategy.toml (including strategy overview)
            if let Err(e) = state.onboarding_manager.save_strategy_full(
                hitting_budget_pct,
                &category_weights,
                None,
                None,
                strategy_overview.as_deref(),
            ) {
                warn!("Failed to save strategy.toml: {}", e);
            } else {
                info!("Saved strategy config to strategy.toml (budget={}%, weights updated)", hitting_budget_pct);
            }

            // Mark strategy as configured and advance to Complete
            state.onboarding_progress.strategy_configured = true;
            state.onboarding_progress.current_step = OnboardingStep::Complete;
            if let Err(e) = state
                .onboarding_manager
                .save_progress(&state.onboarding_progress)
            {
                warn!("Failed to save onboarding progress: {}", e);
            }

            // Rebuild LLM client with new config
            state.llm_client = Arc::new(LlmClient::from_config(&state.config));

            // Transition to Draft mode
            state.app_mode = AppMode::Draft;
            let _ = ui_tx.send(UiUpdate::ModeChanged(AppMode::Draft)).await;
            let snapshot = state.build_snapshot();
            let _ = ui_tx
                .send(UiUpdate::StateSnapshot(Box::new(snapshot)))
                .await;
        }
        OnboardingAction::ConfigureStrategyWithLlm(description) => {
            // Check if LLM is available
            let llm_client = state.llm_client.clone();
            match &*llm_client {
                LlmClient::Disabled => {
                    let _ = ui_tx
                        .send(UiUpdate::OnboardingUpdate(
                            OnboardingUpdate::StrategyLlmError(
                                "LLM is not configured. Please set up an API key first.".to_string(),
                            ),
                        ))
                        .await;
                }
                LlmClient::Active(_) => {
                    // Allocate a unique generation ID BEFORE spawning the task
                    // to prevent race conditions with stale events.
                    let generation = state.llm_requests.allocate_id();
                    let tx = ui_tx.clone();

                    // Build the prompt for strategy configuration
                    let league_ctx = prompt::format_league_context(&state.config.league, state.roster_config.as_ref());
                    let cat_keys: Vec<&str> = state.config.league.batting_categories.categories.iter()
                        .chain(state.config.league.pitching_categories.categories.iter())
                        .map(|s| s.as_str())
                        .collect();
                    let system = format!(
                        "You are a fantasy baseball strategy advisor. Given the user's \
                        strategy description, output ONLY a valid JSON object (no markdown, no \
                        explanation) with exactly these fields:\n\
                        - \"hitting_budget_pct\": integer 0-100 (percentage of budget for hitting)\n\
                        - \"category_weights\": object with keys {cat_keys}, \
                        each a float where 1.0 = normal importance, >1.0 = overweight, <1.0 = underweight (min 0.0, max 5.0)\n\
                        - \"strategy_overview\": a 2-3 sentence prose summary of the strategy that captures the key \
                        decisions (budget split, punt categories, target player profiles, market edges). This will be \
                        fed to the draft-time AI advisor for context.\n\n\
                        {league_ctx}",
                        cat_keys = cat_keys.join(", "),
                        league_ctx = league_ctx,
                    );

                    let user_content = format!(
                        "Configure my draft strategy based on this description:\n\n{}",
                        description
                    );

                    let categories = crate::onboarding::strategy_config::categories_from_league(&state.config.league);

                    // Spawn LLM streaming task
                    let handle = tokio::spawn(async move {
                        let (stream_tx, mut stream_rx) = tokio::sync::mpsc::channel::<crate::protocol::LlmEvent>(100);

                        // Start the LLM stream in a separate task
                        let client = llm_client.clone();
                        let sys = system.to_string();
                        let usr = user_content.clone();
                        tokio::spawn(async move {
                            let _ = client.stream_message(&sys, &usr, 1024, stream_tx, generation).await;
                        });

                        let mut full_text = String::new();

                        while let Some(event) = stream_rx.recv().await {
                            match event {
                                crate::protocol::LlmEvent::Token { text, generation: g } => {
                                    if g == generation {
                                        full_text.push_str(&text);
                                        let _ = tx
                                            .send(UiUpdate::OnboardingUpdate(
                                                OnboardingUpdate::StrategyLlmToken(text),
                                            ))
                                            .await;
                                    }
                                }
                                crate::protocol::LlmEvent::Complete { full_text: ft, generation: g, .. } => {
                                    if g == generation {
                                        full_text = ft;
                                        break;
                                    }
                                }
                                crate::protocol::LlmEvent::Error { message, generation: g } => {
                                    if g == generation {
                                        let _ = tx
                                            .send(UiUpdate::OnboardingUpdate(
                                                OnboardingUpdate::StrategyLlmError(message),
                                            ))
                                            .await;
                                        return;
                                    }
                                }
                            }
                        }

                        // Parse JSON from the response
                        match parse_strategy_json(&full_text, &categories) {
                            Ok((pct, weights, overview)) => {
                                let _ = tx
                                    .send(UiUpdate::OnboardingUpdate(
                                        OnboardingUpdate::StrategyLlmComplete {
                                            hitting_budget_pct: pct,
                                            category_weights: weights,
                                            strategy_overview: overview,
                                        },
                                    ))
                                    .await;
                            }
                            Err(e) => {
                                let _ = tx
                                    .send(UiUpdate::OnboardingUpdate(
                                        OnboardingUpdate::StrategyLlmError(
                                            format!("Failed to parse LLM response: {}", e),
                                        ),
                                    ))
                                    .await;
                            }
                        }
                    });
                    state.llm_requests.track(generation, handle);
                }
            }
        }
        OnboardingAction::TestConnectionWith { .. } => {
            // Only used in settings mode; ignore during onboarding.
        }
        other => {
            // Any unexpected variants during onboarding are handled by the
            // settings path or ignored.
            warn!("Unexpected onboarding action in onboarding handler: {:?}", other);
        }
    }
}

/// Handle an onboarding-style action dispatched from the settings screen.
///
/// Settings mode reuses the same onboarding input handlers, so it receives
/// the same `OnboardingAction` variants. However, the behavior differs:
/// - `SetProvider`/`SetModel`/`SetApiKey` update config and reload the LLM client
/// - `SaveStrategyConfig` persists strategy without transitioning to Draft
/// - `TestConnection` works the same as during onboarding
/// - `GoNext`/`GoBack`/`Skip` are filtered out by input.rs and should not arrive
/// - `ConfigureStrategyWithLlm` works the same as during onboarding
pub(super) async fn handle_settings_action(
    state: &mut AppState,
    action: OnboardingAction,
    ui_tx: &mpsc::Sender<UiUpdate>,
) {
    use wyncast_core::llm::provider::LlmProvider;

    match action {
        OnboardingAction::SaveLlmConfig { provider, model_id, api_key } => {
            // Batch save: update provider, model, and optionally API key in one go.
            state.config.strategy.llm.provider = provider.clone();
            state.config.strategy.llm.model = model_id.clone();
            state.onboarding_progress.llm_provider = Some(provider.clone());
            state.onboarding_progress.llm_model = Some(model_id);

            if let Some(key) = api_key {
                if !key.is_empty() {
                    save_api_key_for_provider(&provider, &key, &mut state.config, &state.onboarding_manager);
                }
            }

            state.reload_llm_client();

            if let Err(e) = state.onboarding_manager.save_progress(&state.onboarding_progress) {
                warn!("Failed to save onboarding progress after SaveLlmConfig: {}", e);
            }

            // Persist LLM settings to strategy.toml
            if let Err(e) = state.onboarding_manager.save_strategy_full(
                (state.config.strategy.hitting_budget_fraction * 100.0) as u8,
                &crate::onboarding::strategy_config::CategoryWeights::from_config_weights(
                    &state.config.strategy.weights,
                    crate::onboarding::strategy_config::categories_from_league(&state.config.league),
                ),
                Some(&state.config.strategy.llm.provider),
                Some(&state.config.strategy.llm.model),
                state.config.strategy.strategy_overview.as_deref(),
            ) {
                warn!("Failed to save strategy.toml after SaveLlmConfig: {}", e);
            } else {
                info!("Settings: saved LLM config (provider={:?}, model={})",
                    state.config.strategy.llm.provider,
                    state.config.strategy.llm.model,
                );
            }

            // Update the saved API key mask for the UI
            let raw_key = get_api_key_for_provider(&provider, &state.config);
            let mask = if raw_key.is_empty() {
                None
            } else {
                let m = crate::onboarding::strategy_config::mask_api_key(&raw_key);
                if m.is_empty() { None } else { Some(m) }
            };
            let _ = ui_tx
                .send(UiUpdate::OnboardingUpdate(
                    OnboardingUpdate::ProgressSync {
                        provider: state.onboarding_progress.llm_provider.clone(),
                        model: state.onboarding_progress.llm_model.clone(),
                        api_key_mask: mask,
                    },
                ))
                .await;

            // Auto-trigger connection test
            Box::pin(handle_settings_action(state, OnboardingAction::TestConnection, ui_tx)).await;
        }
        OnboardingAction::SetProvider(provider) => {
            state.config.strategy.llm.provider = provider.clone();
            state.config.strategy.llm.model = String::new();
            state.onboarding_progress.llm_provider = Some(provider.clone());
            state.onboarding_progress.llm_model = None;
            state.reload_llm_client();
            if let Err(e) = state.onboarding_manager.save_progress(&state.onboarding_progress) {
                warn!("Failed to save onboarding progress after SetProvider: {}", e);
            }
            // Update the saved API key mask for the newly selected provider
            // so the TUI shows the correct mask (or clears it if no key exists).
            let raw_key = get_api_key_for_provider(&provider, &state.config);
            let mask = if raw_key.is_empty() {
                None
            } else {
                let m = crate::onboarding::strategy_config::mask_api_key(&raw_key);
                if m.is_empty() { None } else { Some(m) }
            };
            let _ = ui_tx
                .send(UiUpdate::OnboardingUpdate(
                    OnboardingUpdate::ProgressSync {
                        provider: state.onboarding_progress.llm_provider.clone(),
                        model: state.onboarding_progress.llm_model.clone(),
                        api_key_mask: mask,
                    },
                ))
                .await;
        }
        OnboardingAction::SetModel(model_id) => {
            state.config.strategy.llm.model = model_id.clone();
            state.onboarding_progress.llm_model = Some(model_id);
            state.reload_llm_client();
            if let Err(e) = state.onboarding_manager.save_progress(&state.onboarding_progress) {
                warn!("Failed to save onboarding progress after SetModel: {}", e);
            }
        }
        OnboardingAction::SetApiKey(key) => {
            let provider = state
                .onboarding_progress
                .llm_provider
                .clone()
                .unwrap_or(LlmProvider::Anthropic);
            save_api_key_for_provider(&provider, &key, &mut state.config, &state.onboarding_manager);
            // Reload LLM client so the new key takes effect immediately
            state.reload_llm_client();
            // Auto-trigger connection test so the user doesn't need a second Enter.
            Box::pin(handle_settings_action(state, OnboardingAction::TestConnection, ui_tx)).await;
        }
        OnboardingAction::TestConnectionWith { provider, model_id, api_key } => {
            // Test with explicit params — does NOT mutate app state.
            // Used by the settings cascade so Esc can cleanly revert.
            if api_key.is_empty() {
                let _ = ui_tx
                    .send(UiUpdate::OnboardingUpdate(
                        OnboardingUpdate::ConnectionTestResult {
                            success: false,
                            message: "No API key entered".to_string(),
                        },
                    ))
                    .await;
                return;
            }
            let tx = ui_tx.clone();
            tokio::spawn(async move {
                let result = test_api_connection(&provider, &api_key, &model_id).await;
                let _ = tx
                    .send(UiUpdate::OnboardingUpdate(
                        OnboardingUpdate::ConnectionTestResult {
                            success: result.is_ok(),
                            message: match &result {
                                Ok(msg) => msg.clone(),
                                Err(msg) => msg.clone(),
                            },
                        },
                    ))
                    .await;
            });
        }
        OnboardingAction::TestConnection => {
            // Same as onboarding: spawn async test
            let provider = state
                .onboarding_progress
                .llm_provider
                .clone()
                .unwrap_or(LlmProvider::Anthropic);
            let model_id = state
                .onboarding_progress
                .llm_model
                .clone()
                .unwrap_or_else(|| state.config.strategy.llm.model.clone());

            let api_key = get_api_key_for_provider(&provider, &state.config);

            if api_key.is_empty() {
                let _ = ui_tx
                    .send(UiUpdate::OnboardingUpdate(
                        OnboardingUpdate::ConnectionTestResult {
                            success: false,
                            message: "No API key entered".to_string(),
                        },
                    ))
                    .await;
                return;
            }

            let tx = ui_tx.clone();
            tokio::spawn(async move {
                let result = test_api_connection(&provider, &api_key, &model_id).await;
                let _ = tx
                    .send(UiUpdate::OnboardingUpdate(
                        OnboardingUpdate::ConnectionTestResult {
                            success: result.is_ok(),
                            message: match &result {
                                Ok(msg) => msg.clone(),
                                Err(msg) => msg.clone(),
                            },
                        },
                    ))
                    .await;
            });
        }
        OnboardingAction::SaveStrategyConfig { hitting_budget_pct, category_weights, strategy_overview } => {
            // Update in-memory config
            state.config.strategy.hitting_budget_fraction = hitting_budget_pct as f64 / 100.0;
            state.config.strategy.weights = category_weights.to_config_weights();
            state.config.strategy.strategy_overview = strategy_overview.clone();

            // Persist to strategy.toml (including current LLM provider/model
            // in case they were changed on the LLM tab)
            if let Err(e) = state.onboarding_manager.save_strategy_full(
                hitting_budget_pct,
                &category_weights,
                Some(&state.config.strategy.llm.provider),
                Some(&state.config.strategy.llm.model),
                strategy_overview.as_deref(),
            ) {
                warn!("Failed to save strategy.toml: {}", e);
            } else {
                info!("Settings: saved strategy config (budget={}%, weights updated)", hitting_budget_pct);
            }

            // Reload LLM client in case provider/model changed on the LLM tab
            state.reload_llm_client();

            // Recalculate valuations with new strategy weights
            let roster = state.roster_config.clone().unwrap_or_else(super::AppState::default_roster_config);
            valuation::recalculate_all(
                &mut state.available_players,
                &roster,
                &state.config.league,
                &state.config.strategy,
                &state.draft_state,
                &state.stat_registry,
            );
            state.scarcity = compute_scarcity(&state.available_players, &roster);

            // Send updated snapshot to TUI (stay in Settings mode)
            let snapshot = state.build_snapshot();
            let _ = ui_tx
                .send(UiUpdate::StateSnapshot(Box::new(snapshot)))
                .await;
        }
        OnboardingAction::ConfigureStrategyWithLlm(description) => {
            // Delegate to the same LLM generation logic as onboarding
            handle_onboarding_action(
                state,
                OnboardingAction::ConfigureStrategyWithLlm(description),
                ui_tx,
            )
            .await;
        }
        // GoNext/GoBack/Skip are filtered by the input handler and should
        // not reach here. If they do, ignore them silently.
        OnboardingAction::GoNext | OnboardingAction::GoBack | OnboardingAction::Skip => {}
    }
}

/// Persist an API key for the given provider to both in-memory config and
/// the `credentials.toml` file via the OnboardingManager's FileSystem trait.
pub(super) fn save_api_key_for_provider(
    provider: &wyncast_core::llm::provider::LlmProvider,
    key: &str,
    config: &mut Config,
    onboarding_manager: &crate::onboarding::OnboardingManager<crate::onboarding::RealFileSystem>,
) {
    use wyncast_core::llm::provider::LlmProvider;

    match provider {
        LlmProvider::Anthropic => {
            config.credentials.anthropic_api_key = Some(key.to_string());
        }
        LlmProvider::Google => {
            config.credentials.google_api_key = Some(key.to_string());
        }
        LlmProvider::OpenAI => {
            config.credentials.openai_api_key = Some(key.to_string());
        }
    }

    if let Err(e) = onboarding_manager.save_credentials(&config.credentials) {
        warn!("Failed to save credentials.toml: {}", e);
    } else {
        info!("Saved API key for {} to credentials.toml", provider.display_name());
    }
}

/// Get the API key for a given provider from the current config.
pub(super) fn get_api_key_for_provider(
    provider: &wyncast_core::llm::provider::LlmProvider,
    config: &Config,
) -> String {
    use wyncast_core::llm::provider::LlmProvider;

    match provider {
        LlmProvider::Anthropic => config
            .credentials
            .anthropic_api_key
            .clone()
            .unwrap_or_default(),
        LlmProvider::Google => config
            .credentials
            .google_api_key
            .clone()
            .unwrap_or_default(),
        LlmProvider::OpenAI => config
            .credentials
            .openai_api_key
            .clone()
            .unwrap_or_default(),
    }
}

/// Test an API connection by making a minimal request to the provider.
///
/// Returns `Ok(message)` on success or `Err(message)` on failure.
pub(super) async fn test_api_connection(
    provider: &wyncast_core::llm::provider::LlmProvider,
    api_key: &str,
    model_id: &str,
) -> Result<String, String> {
    use wyncast_core::llm::provider::LlmProvider;

    let client = reqwest::Client::new();

    match provider {
        LlmProvider::Anthropic => {
            let resp = client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&serde_json::json!({
                    "model": model_id,
                    "max_tokens": 1,
                    "messages": [{"role": "user", "content": "hi"}]
                }))
                .send()
                .await
                .map_err(|e| format!("Connection error: {}", e))?;

            let status = resp.status();
            if status.is_success() {
                Ok("Connected to Anthropic API".to_string())
            } else if status.as_u16() == 401 || status.as_u16() == 403 {
                Err("Invalid API key".to_string())
            } else {
                let body = resp.text().await.unwrap_or_default();
                Err(format!("HTTP {}: {}", status, body.chars().take(100).collect::<String>()))
            }
        }
        LlmProvider::Google => {
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
                model_id, api_key
            );
            let resp = client
                .post(&url)
                .header("content-type", "application/json")
                .json(&serde_json::json!({
                    "contents": [{"parts": [{"text": "hi"}]}],
                    "generationConfig": {"maxOutputTokens": 1}
                }))
                .send()
                .await
                .map_err(|e| format!("Connection error: {}", e))?;

            let status = resp.status();
            if status.is_success() {
                Ok("Connected to Google Gemini API".to_string())
            } else if status.as_u16() == 400 || status.as_u16() == 401 || status.as_u16() == 403 {
                Err("Invalid API key".to_string())
            } else {
                let body = resp.text().await.unwrap_or_default();
                Err(format!("HTTP {}: {}", status, body.chars().take(100).collect::<String>()))
            }
        }
        LlmProvider::OpenAI => {
            let resp = client
                .post("https://api.openai.com/v1/chat/completions")
                .header("Authorization", format!("Bearer {}", api_key))
                .header("content-type", "application/json")
                .json(&serde_json::json!({
                    "model": model_id,
                    "max_tokens": 1,
                    "messages": [{"role": "user", "content": "hi"}]
                }))
                .send()
                .await
                .map_err(|e| format!("Connection error: {}", e))?;

            let status = resp.status();
            if status.is_success() {
                Ok("Connected to OpenAI API".to_string())
            } else if status.as_u16() == 401 || status.as_u16() == 403 {
                Err("Invalid API key".to_string())
            } else {
                let body = resp.text().await.unwrap_or_default();
                Err(format!("HTTP {}: {}", status, body.chars().take(100).collect::<String>()))
            }
        }
    }
}

/// Parse the LLM's JSON response into a hitting budget percentage, category weights,
/// and strategy overview.
///
/// The LLM is prompted to return a JSON object with `hitting_budget_pct` (int 0-100),
/// `category_weights` (map of category name to float), and `strategy_overview` (string).
/// This function extracts the JSON from the response (stripping any surrounding
/// text/markdown fences) and parses it.
pub(super) fn parse_strategy_json(
    text: &str,
    categories: &[String],
) -> Result<(u8, crate::onboarding::strategy_config::CategoryWeights, String), String> {
    use crate::onboarding::strategy_config::CategoryWeights;

    // Strip markdown code fences if present.
    // Use safe string operations to avoid panics on edge cases.
    let trimmed = text.trim();
    let json_str = if let Some(after_backticks) = trimmed.strip_prefix("```") {
        // Find end of the opening fence line (e.g. "```json\n")
        let after_fence = if let Some(newline_pos) = after_backticks.find('\n') {
            &after_backticks[newline_pos + 1..]
        } else {
            // Opening fence with no newline — nothing left after the fence marker
            after_backticks
        };
        // Strip the closing ``` if present
        if let Some(close_pos) = after_fence.rfind("```") {
            &after_fence[..close_pos]
        } else {
            after_fence
        }
    } else {
        trimmed
    };

    // Try to find a JSON object in the text (between first { and last })
    let json_str = if let (Some(start), Some(end)) = (json_str.find('{'), json_str.rfind('}')) {
        &json_str[start..=end]
    } else {
        return Err("No JSON object found in response".to_string());
    };

    let parsed: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| format!("JSON parse error: {}", e))?;

    let pct = parsed
        .get("hitting_budget_pct")
        .and_then(|v| v.as_u64())
        .map(|v| v.min(100) as u8)
        .unwrap_or(65);

    let mut weights = CategoryWeights::new(categories.to_vec());

    if let Some(cw) = parsed.get("category_weights").and_then(|v| v.as_object()) {
        for (idx, name) in categories.iter().enumerate() {
            if let Some(val) = cw.get(name.as_str()).and_then(|v| v.as_f64()) {
                let clamped = val.clamp(0.0, 5.0) as f32;
                weights.set(idx, clamped);
            }
        }
    }

    let overview = parsed
        .get("strategy_overview")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    Ok((pct, weights, overview))
}
