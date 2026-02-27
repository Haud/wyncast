// Library root: re-exports all modules so integration tests and external
// consumers can access the crate's public API.

pub mod app;
pub mod config;
pub mod db;
pub mod draft;
pub mod llm;
pub mod protocol;
pub mod tui;
pub mod valuation;
pub mod ws_server;
