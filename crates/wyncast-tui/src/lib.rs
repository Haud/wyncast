// Library root: re-exports all modules so integration tests and external
// consumers can access the crate's public API.

// Modules remaining in wyncast-tui
pub mod llm;
pub mod tui;

// Re-exports from wyncast-core for backward-compat within this crate's tests
pub use wyncast_core::app_dirs;
pub use wyncast_core::config;
pub use wyncast_core::db;
pub use wyncast_core::migrations;
pub use wyncast_core::picks;
pub use wyncast_core::stats;
pub use wyncast_core::ws_server;

// Re-exports from wyncast-baseball for backward-compat (modules moved there)
pub use wyncast_baseball::draft;
pub use wyncast_baseball::matchup;
pub use wyncast_baseball::valuation;

// Re-exports from wyncast-app for backward-compat
pub use wyncast_app::app;
pub use wyncast_app::onboarding;
pub use wyncast_app::protocol;

#[cfg(test)]
pub mod test_utils;
