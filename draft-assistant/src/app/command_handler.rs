use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::protocol::{
    AppMode, OnboardingAction, OnboardingUpdate, UiUpdate, UserCommand,
};

use super::AppState;
use super::onboarding_handler::{get_api_key_for_provider, handle_onboarding_action, handle_settings_action};

/// Handle a user command from the TUI.
pub(super) async fn handle_user_command(
    state: &mut AppState,
    cmd: UserCommand,
    ui_tx: &mpsc::Sender<UiUpdate>,
) {
    match cmd {
        UserCommand::SwitchTab(tab) => {
            state.active_tab = tab;
            info!("Switched to tab: {:?}", tab);
        }
        UserCommand::RequestKeyframe => {
            info!("Manual keyframe refresh requested");
            if let Some(ref ws_tx) = state.ws_outbound_tx {
                let request = serde_json::json!({
                    "type": "REQUEST_KEYFRAME"
                });
                if let Err(e) = ws_tx.send(request.to_string()).await {
                    warn!("Failed to send REQUEST_KEYFRAME: {}", e);
                }
            } else {
                warn!("Cannot request keyframe: no outbound WebSocket channel");
            }
        }
        UserCommand::ManualPick {
            player_name,
            team_idx,
            price,
        } => {
            info!(
                "Manual pick: {} -> team {} for ${}",
                player_name, team_idx, price
            );
            if team_idx < state.draft_state.teams.len() {
                let team = &state.draft_state.teams[team_idx];
                let pick = crate::draft::pick::DraftPick {
                    pick_number: 0, // overwritten by record_pick
                    team_id: team.team_id.clone(),
                    team_name: team.team_name.clone(),
                    player_name,
                    position: "UTIL".to_string(),
                    price,
                    espn_player_id: None,
                    eligible_slots: vec![],
            assigned_slot: None,
                };
                state.process_new_picks(vec![pick]);

                // Send updated state to TUI
                let snapshot = state.build_snapshot();
                let _ = ui_tx
                    .send(UiUpdate::StateSnapshot(Box::new(snapshot)))
                    .await;
            }
        }
        UserCommand::Scroll { .. } => {
            // Scroll is handled by the TUI directly, no app-level action needed
        }
        UserCommand::OnboardingAction(action) => {
            let in_settings = matches!(state.app_mode, AppMode::Settings(_));
            match &action {
                OnboardingAction::SetApiKey(_) => info!("Onboarding action: SetApiKey(***)"),
                _ => info!("Onboarding action: {:?}", action),
            }
            if in_settings {
                handle_settings_action(state, action, ui_tx).await;
            } else {
                handle_onboarding_action(state, action, ui_tx).await;
            }
        }
        UserCommand::OpenSettings => {
            info!("Opening settings screen");
            state.app_mode = AppMode::Settings(crate::protocol::SettingsSection::LlmConfig);
            let _ = ui_tx
                .send(UiUpdate::ModeChanged(AppMode::Settings(
                    crate::protocol::SettingsSection::LlmConfig,
                )))
                .await;
            // Send a ProgressSync so the TUI can show a masked placeholder
            // for the saved API key (instead of showing a blank field).
            let provider = state
                .onboarding_progress
                .llm_provider
                .clone()
                .unwrap_or(crate::llm::provider::LlmProvider::Anthropic);
            let raw_key = get_api_key_for_provider(&provider, &state.config);
            let mask = if raw_key.is_empty() {
                None
            } else {
                let m = crate::tui::onboarding::llm_setup::mask_api_key(&raw_key);
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
        UserCommand::ExitSettings => {
            info!("Exiting settings, returning to draft mode");
            state.app_mode = AppMode::Draft;
            let _ = ui_tx.send(UiUpdate::ModeChanged(AppMode::Draft)).await;
            let snapshot = state.build_snapshot();
            let _ = ui_tx
                .send(UiUpdate::StateSnapshot(Box::new(snapshot)))
                .await;
        }
        UserCommand::SaveAndExitSettings { llm, strategy } => {
            info!("Saving settings and exiting to draft mode");
            // Save LLM config if dirty
            if let Some((provider, model_id, api_key)) = llm {
                handle_settings_action(
                    state,
                    OnboardingAction::SaveLlmConfig { provider, model_id, api_key },
                    ui_tx,
                )
                .await;
            }
            // Save strategy config if dirty
            if let Some((hitting_budget_pct, category_weights, strategy_overview)) = strategy {
                handle_settings_action(
                    state,
                    OnboardingAction::SaveStrategyConfig {
                        hitting_budget_pct,
                        category_weights,
                        strategy_overview,
                    },
                    ui_tx,
                )
                .await;
            }
            // Transition to draft mode
            state.app_mode = AppMode::Draft;
            let _ = ui_tx.send(UiUpdate::ModeChanged(AppMode::Draft)).await;
            let snapshot = state.build_snapshot();
            let _ = ui_tx
                .send(UiUpdate::StateSnapshot(Box::new(snapshot)))
                .await;
        }
        UserCommand::SwitchSettingsTab(section) => {
            state.app_mode = AppMode::Settings(section);
            let _ = ui_tx
                .send(UiUpdate::ModeChanged(AppMode::Settings(section)))
                .await;
            // When switching to the StrategyConfig tab, send current saved
            // config so the TUI initializes the strategy wizard at the
            // Review step with the correct values (including strategy_overview).
            if section == crate::protocol::SettingsSection::StrategyConfig {
                let pct = (state.config.strategy.hitting_budget_fraction * 100.0).round() as u8;
                let weights = crate::tui::onboarding::strategy_setup::CategoryWeights::from_config_weights(
                    &state.config.strategy.weights,
                    crate::tui::onboarding::strategy_setup::categories_from_league(&state.config.league),
                );
                let overview = state.config.strategy.strategy_overview.clone().unwrap_or_default();
                let _ = ui_tx
                    .send(UiUpdate::OnboardingUpdate(
                        crate::protocol::OnboardingUpdate::StrategyLlmComplete {
                            hitting_budget_pct: pct,
                            category_weights: weights,
                            strategy_overview: overview,
                        },
                    ))
                    .await;
            }
        }
        UserCommand::Quit => {
            // Handled in the main loop
        }
    }
}
