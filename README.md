# Wyndham Draft Assistant

A real-time fantasy baseball draft assistant for the **Wyndham Lewis Vorticist Baseball** ESPN league (10-team, H2H Most Categories, salary cap auction).

Two-component system:
- **Rust TUI backend** — valuation engine, real-time analysis (Claude API), terminal dashboard
- **Firefox WebExtension** — scrapes ESPN draft page and relays state to the backend via WebSocket

## Prerequisites

- [Rust](https://rustup.rs/) 1.74+ (edition 2021)
- [Firefox](https://www.mozilla.org/firefox/) 109+ (for the extension; Developer Edition recommended)
- An [Anthropic API key](https://console.anthropic.com/) (optional — enables Claude-powered draft analysis)
- [just](https://github.com/casey/just) (optional — for the `justfile` recipes)

## Quickstart

```bash
# Build the Rust backend
cargo build --workspace

# Run the TUI
cargo run -p wyncast-tui

# Run all tests
cargo test --workspace

# Build and lint
cargo clippy --workspace -- -D warnings
```

Or using `just`:

```bash
just build    # Build everything (Rust + extension)
just run      # Run the TUI app
just test     # Run all tests
just check    # Clippy + fmt check
just release  # Release build
```

## Directory Layout

```
.
├── Cargo.toml          # Workspace manifest
├── Cargo.lock
├── justfile            # Build recipes
├── crates/
│   └── wyncast-tui/    # Main Rust crate (TUI + valuation engine + LLM)
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs
│           ├── lib.rs
│           ├── app/         # Central event loop and state
│           ├── config.rs    # TOML config loading
│           ├── db.rs        # SQLite persistence
│           ├── ws_server.rs # WebSocket server
│           ├── protocol.rs  # Message protocol definitions
│           ├── valuation/   # Player valuation pipeline
│           ├── llm/         # Claude API integration
│           ├── tui/         # Terminal UI (ratatui)
│           └── draft/       # Draft state management
├── extension/          # Firefox WebExtension (peer artifact)
│   ├── manifest.json
│   ├── background.js
│   ├── content_scripts/
│   └── chrome/         # Chrome variant
├── migrations/         # SQLite schema migrations (embedded at compile time)
│   ├── up/
│   └── down/
├── projections/        # Projection CSV files (not in git)
├── scripts/            # Utility scripts
└── docs/               # Design documents
    └── design/
```

## Configuration

Config files are auto-generated on first run in the OS-standard app data directory
(e.g. `~/.local/share/wyncast/config/` on Linux). Three TOML files:

- `league.toml` — league structure, teams, roster slots, scoring categories
- `strategy.toml` — valuation weights, budget split, LLM settings, data paths
- `credentials.toml` — API key for Claude (optional)

Configure team names and your team ID in `league.toml` before draft day.

## Projection Data

Projection CSV files are **not** checked into git. Place them at the paths configured
in `strategy.toml`. See the existing `projections/` directory for the expected format.

## Installing the Firefox Extension

1. Open Firefox → `about:debugging#/runtime/this-firefox`
2. Click **"Load Temporary Add-on..."**
3. Select `extension/manifest.json`

The extension connects to `ws://localhost:9001`. Start the Rust backend first.

## Logging

Logs go to `~/.local/share/wyncast/logs/draft-assistant.log` (not the terminal — that's the TUI).

Default level is `INFO`. Override with `RUST_LOG=debug cargo run -p wyncast-tui`.
