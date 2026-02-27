# Wyndham Draft Assistant

A real-time fantasy baseball draft assistant for the **Wyndham Lewis Vorticist Baseball** ESPN league (10-team, H2H Most Categories, salary cap auction).

Two-component system:
- **Rust TUI backend** — valuation engine, real-time analysis (Claude API), terminal dashboard
- **Firefox WebExtension** — scrapes ESPN draft page and relays state to the backend via WebSocket

## Prerequisites

- [Rust](https://rustup.rs/) 1.74+ (edition 2021; required by ratatui/crossterm dependencies)
- [Firefox](https://www.mozilla.org/firefox/) 109+ (for the extension; Developer Edition recommended)
- An [Anthropic API key](https://console.anthropic.com/) (optional — enables Claude-powered draft analysis)

## Building

```bash
cd draft-assistant

# Debug build
cargo build

# Release build (recommended for draft day)
cargo build --release

# Run tests
cargo test
```

The binary is output to `target/release/draft-assistant` (or `draft-assistant.exe` on Windows).

## Configuration

All config lives in `draft-assistant/config/`. Three TOML files:

### 1. `config/league.toml` (required)

League structure — teams, roster slots, scoring categories. This file ships pre-configured for the Wyndham league with all required sections filled in. The only sections you need to customize before draft day are the team names and your team ID:

```toml
# These are the only sections you need to edit.
# All other sections (roster, categories, roster_limits, etc.) are pre-configured.

[league.teams]
team_1 = "Your Team Name"
team_2 = "Opponent 2"
# ... (populate all 10 from ESPN)

[league.my_team]
team_id = "team_1"  # Must match your key in [league.teams]
```

### 2. `config/strategy.toml` (required)

Valuation weights, budget split, LLM settings, data paths. Ships with sensible defaults. Key knobs:

| Section | Key | Default | Purpose |
|---------|-----|---------|---------|
| `[budget]` | `hitting_budget_fraction` | `0.65` | 65% hitting / 35% pitching budget split |
| `[category_weights]` | `SV` | `0.7` | Soft-punt saves (reduce to devalue closers) |
| `[category_weights]` | `BB`, `HD` | `1.0` | Increase to 1.1–1.3 for market edge |
| `[llm]` | `model` | *(see strategy.toml)* | Claude model for analysis |
| `[llm]` | `analysis_trigger` | `"nomination"` | `"nomination"` = every nomination, `"my_turn_only"` = only yours |
| `[websocket]` | `port` | `9001` | WebSocket port (must match extension) |
| `[data_paths]` | various | `data/...` | Paths to projection CSVs (relative to cwd) |

### 3. `config/credentials.toml` (optional)

API key for Claude-powered analysis. **Not checked into git.**

```bash
cp config/credentials.toml.example config/credentials.toml
```

Then edit it:

```toml
anthropic_api_key = "sk-ant-your-key-here"
```

If omitted, the app runs with LLM features disabled (valuations and the TUI still work).

## Projection Data

You need to supply your own projection CSV files. These are **not** checked into git. Place them at the paths configured in `strategy.toml` (defaults shown below).

### Required files

**`data/projections/hitters.csv`**

```csv
Name,Team,PA,AB,H,HR,R,RBI,BB,SB,AVG
Aaron Judge,NYY,700,600,180,52,120,130,90,5,0.300
```

Column `AVG` can also be named `BA`.

**`data/projections/pitchers_sp.csv`**

```csv
Name,Team,IP,K,W,SV,ERA,WHIP,G,GS
Gerrit Cole,NYY,200.0,250,16,0,2.80,1.05,32,32
```

Column `K` can also be named `SO`.

**`data/projections/pitchers_rp.csv`**

Same format as SP. Can include an optional `HD` column for holds.

```csv
Name,Team,IP,K,W,SV,HD,ERA,WHIP,G,GS
Emmanuel Clase,CLE,70.0,80,5,40,0,1.90,0.88,70,0
```

**`data/adp.csv`**

```csv
Name,ADP
Aaron Judge,2.5
Mookie Betts,5.8
```

### Optional files

**`data/holds_projections.csv`** — overrides RP holds values. If a reliever isn't in this file and has no `HD` in the RP CSV, holds are estimated as `(G - SV - GS) * default_hold_rate`.

```csv
Name,Team,HD
Devin Williams,NYY,25
Clay Holmes,CLE,18
```

### Where to get projections

Use any major projection system (Steamer, ZiPS, ATC, PECOTA, etc.). Export or copy into the CSV format above. Player names must match across all files.

## Running

```bash
cd draft-assistant
cargo run --release
```

On startup the app will:
1. Load config from `config/`
2. Load projections and compute valuations (z-scores → VOR → auction dollars)
3. Open/create SQLite database (path from `[database].path` in `strategy.toml`, default: `draft-assistant.db`)
4. Start WebSocket server on `127.0.0.1:9001`
5. Launch the TUI dashboard

Press `q` or `Ctrl+C` to quit. State is persisted to the database and restored on next launch.

## Installing the Firefox Extension

1. Open Firefox and navigate to `about:debugging#/runtime/this-firefox`
2. Click **"Load Temporary Add-on..."**
3. Select `extension/manifest.json` inside the `draft-assistant/` directory

The extension automatically connects to `ws://localhost:9001` with exponential backoff. Start the Rust backend first.

**During a draft:** navigate to your ESPN draft page. The content script will extract draft state (picks, nominations, bids) and forward it to the backend in real time.

To verify the connection, check the browser console for messages prefixed with `[WyndhamDraftSync:BG]`.

> **Note:** Temporary extensions are removed when Firefox closes. You'll need to reload it each session from `about:debugging`.

## Logging

Logs go to `draft-assistant/logs/draft-assistant.log` (not the terminal — that's the TUI).

Default level is `INFO`. Override with the `RUST_LOG` environment variable:

```bash
RUST_LOG=debug cargo run --release
```

## TUI Keyboard Controls

| Key | Action |
|-----|--------|
| `1`–`5` | Switch tabs (LLM Analysis, Nomination Plan, Available Players, Draft Log, Teams) |
| `j` / `Down` | Scroll down |
| `k` / `Up` | Scroll up |
| `PageDown` / `PageUp` | Scroll by page |
| `/` | Enter filter mode (type to filter, `Enter` to apply, `Esc` to clear) |
| `p` | Cycle position filter (C → 1B → 2B → ... → RP → All) |
| `r` | Refresh LLM analysis |
| `n` | Refresh nomination plan |
| `Esc` | Clear filters |
| `q` / `Ctrl+C` | Quit |

## Troubleshooting

**App won't start — "config file not found"**
Run from the `draft-assistant/` directory so relative paths resolve correctly.

**"failed to load projections"**
Verify CSV files exist at the paths in `strategy.toml` and have the correct column headers.

**Extension won't connect**
- Start the Rust backend first
- Confirm port 9001 isn't in use: `netstat -an | grep 9001`
- Check `about:debugging` to verify the extension is loaded
- Check browser console for WebSocket errors

**LLM analysis not appearing**
- Verify `config/credentials.toml` exists with a valid API key
- Check `logs/draft-assistant.log` for API errors

**Database locked**
Another instance may be running. Kill it, or delete `draft-assistant.db` to start fresh.

## Project Structure

```
draft-assistant/
├── src/
│   ├── main.rs              # Entry point and startup sequence
│   ├── lib.rs               # Library root (module re-exports)
│   ├── config.rs            # TOML config loading and validation
│   ├── app.rs               # Central event loop and state management
│   ├── db.rs                # SQLite persistence (WAL mode, crash recovery)
│   ├── ws_server.rs         # WebSocket server (tokio-tungstenite)
│   ├── protocol.rs          # Message protocol definitions
│   ├── valuation/           # Player valuation pipeline
│   │   ├── projections.rs   #   CSV data loading
│   │   ├── zscore.rs        #   Z-score computation
│   │   ├── vor.rs           #   Value Over Replacement
│   │   ├── auction.rs       #   Auction dollar conversion + inflation
│   │   ├── scarcity.rs      #   Positional scarcity index
│   │   └── analysis.rs      #   Real-time player analysis engine
│   ├── llm/                 # Claude API integration
│   │   ├── client.rs        #   Streaming SSE client
│   │   └── prompt.rs        #   Prompt construction
│   ├── tui/                 # Terminal UI (ratatui)
│   │   ├── input.rs         #   Keyboard input handling
│   │   ├── layout.rs        #   Dashboard layout
│   │   └── widgets/         #   Individual panels
│   └── draft/               # Draft state management
│       ├── state.rs         #   DraftState tracking
│       ├── pick.rs          #   DraftPick and Position types
│       └── roster.rs        #   Roster slot tracking
├── config/                  # Configuration files
│   ├── league.toml
│   ├── strategy.toml
│   └── credentials.toml.example
├── data/                    # Projection data (not in git)
│   ├── projections/
│   │   ├── hitters.csv
│   │   ├── pitchers_sp.csv
│   │   └── pitchers_rp.csv
│   ├── holds_projections.csv
│   └── adp.csv
├── extension/               # Firefox WebExtension
│   ├── manifest.json
│   ├── background.js
│   └── content_scripts/
│       └── espn.js
└── tests/                   # Integration tests
```
