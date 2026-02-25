// SQLite persistence layer for draft state.

use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

/// A draft pick recorded during the auction.
#[derive(Debug, Clone)]
pub struct DraftPick {
    pub pick_number: u32,
    pub team_id: String,
    pub team_name: String,
    pub player_name: String,
    pub position: String,
    pub price: u32,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// SQLite-backed persistence for players, projections, draft picks, and
/// key-value draft state.
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Open (or create) a SQLite database at `path` and ensure all tables
    /// exist. Pass `":memory:"` for an ephemeral in-memory database (useful
    /// for tests).
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open database at {path}"))?;

        // Foreign keys are left as documentation in the schema but not
        // enforced, because draft_picks stores player_id = 0 as a sentinel
        // for deferred player linkage.
        conn.execute_batch("PRAGMA foreign_keys = OFF;")
            .context("failed to set PRAGMA foreign_keys")?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS players (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT NOT NULL,
                team        TEXT NOT NULL,
                positions   TEXT NOT NULL,
                player_type TEXT NOT NULL,
                UNIQUE(name, team)
            );

            CREATE TABLE IF NOT EXISTS projections (
                player_id INTEGER REFERENCES players(id),
                source    TEXT NOT NULL,
                stat_name TEXT NOT NULL,
                value     REAL NOT NULL,
                PRIMARY KEY (player_id, source, stat_name)
            );

            CREATE TABLE IF NOT EXISTS draft_picks (
                pick_number INTEGER PRIMARY KEY,
                team_id     TEXT NOT NULL,
                team_name   TEXT NOT NULL,
                player_id   INTEGER REFERENCES players(id),
                player_name TEXT NOT NULL,
                position    TEXT NOT NULL,
                price       INTEGER NOT NULL,
                timestamp   TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS draft_state (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            ",
        )
        .context("failed to create database schema")?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Record a single draft pick. Stores `0` for `player_id` (player
    /// linkage is deferred).
    pub fn record_pick(&self, pick: &DraftPick) -> Result<()> {
        let conn = self.conn.lock().expect("database mutex poisoned");
        conn.execute(
            "INSERT INTO draft_picks (pick_number, team_id, team_name, player_id, player_name, position, price, timestamp)
             VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6, ?7)",
            params![
                pick.pick_number,
                pick.team_id,
                pick.team_name,
                pick.player_name,
                pick.position,
                pick.price,
                pick.timestamp.to_rfc3339(),
            ],
        )
        .context("failed to record draft pick")?;
        Ok(())
    }

    /// Load all recorded draft picks, ordered by pick number.
    pub fn load_picks(&self) -> Result<Vec<DraftPick>> {
        let conn = self.conn.lock().expect("database mutex poisoned");
        let mut stmt = conn
            .prepare(
                "SELECT pick_number, team_id, team_name, player_name, position, price, timestamp
                 FROM draft_picks ORDER BY pick_number",
            )
            .context("failed to prepare load_picks query")?;

        let picks = stmt
            .query_map([], |row| {
                let ts_str: String = row.get(6)?;
                let timestamp = chrono::DateTime::parse_from_rfc3339(&ts_str)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            6,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;
                Ok(DraftPick {
                    pick_number: row.get(0)?,
                    team_id: row.get(1)?,
                    team_name: row.get(2)?,
                    player_name: row.get(3)?,
                    position: row.get(4)?,
                    price: row.get(5)?,
                    timestamp,
                })
            })
            .context("failed to query draft picks")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to map draft pick rows")?;

        Ok(picks)
    }

    /// Persist an arbitrary JSON value under `key`. Uses INSERT OR REPLACE so
    /// repeated saves overwrite the previous value.
    pub fn save_state(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        let conn = self.conn.lock().expect("database mutex poisoned");
        let json_str =
            serde_json::to_string(value).context("failed to serialize state value")?;
        conn.execute(
            "INSERT OR REPLACE INTO draft_state (key, value) VALUES (?1, ?2)",
            params![key, json_str],
        )
        .context("failed to save state")?;
        Ok(())
    }

    /// Load a previously saved JSON value by `key`. Returns `None` if the key
    /// does not exist.
    pub fn load_state(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let conn = self.conn.lock().expect("database mutex poisoned");
        let mut stmt = conn
            .prepare("SELECT value FROM draft_state WHERE key = ?1")
            .context("failed to prepare load_state query")?;

        let mut rows = stmt
            .query_map(params![key], |row| {
                let json_str: String = row.get(0)?;
                Ok(json_str)
            })
            .context("failed to query draft state")?;

        match rows.next() {
            Some(row_result) => {
                let json_str = row_result.context("failed to read state row")?;
                let value: serde_json::Value = serde_json::from_str(&json_str)
                    .context("failed to deserialize state value")?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// Insert a player or update their record if a `(name, team)` row already
    /// exists. Returns the player's row id.
    ///
    /// `positions` is stored as a JSON array string (e.g. `["SS","2B"]`).
    pub fn upsert_player(
        &self,
        name: &str,
        team: &str,
        positions: &[String],
        player_type: &str,
    ) -> Result<i64> {
        let conn = self.conn.lock().expect("database mutex poisoned");
        let positions_json = serde_json::to_string(positions)
            .context("failed to serialize positions")?;

        conn.execute(
            "INSERT INTO players (name, team, positions, player_type)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(name, team) DO UPDATE SET
                positions   = excluded.positions,
                player_type = excluded.player_type",
            params![name, team, positions_json, player_type],
        )
        .context("failed to upsert player")?;

        let id = conn.last_insert_rowid();

        // When ON CONFLICT triggers an UPDATE, last_insert_rowid() may return
        // 0 (no new row inserted). In that case, look up the existing id.
        if id == 0 {
            let existing_id: i64 = conn
                .query_row(
                    "SELECT id FROM players WHERE name = ?1 AND team = ?2",
                    params![name, team],
                    |row| row.get(0),
                )
                .context("failed to look up existing player id")?;
            Ok(existing_id)
        } else {
            Ok(id)
        }
    }

    /// Insert a single projection row. Uses INSERT OR REPLACE so re-importing
    /// projections overwrites prior values.
    pub fn insert_projection(
        &self,
        player_id: i64,
        source: &str,
        stat_name: &str,
        value: f64,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("database mutex poisoned");
        conn.execute(
            "INSERT OR REPLACE INTO projections (player_id, source, stat_name, value)
             VALUES (?1, ?2, ?3, ?4)",
            params![player_id, source, stat_name, value],
        )
        .context("failed to insert projection")?;
        Ok(())
    }

    /// Returns `true` if at least one draft pick has been recorded.
    pub fn has_draft_in_progress(&self) -> Result<bool> {
        let conn = self.conn.lock().expect("database mutex poisoned");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM draft_picks", [], |row| row.get(0))
            .context("failed to check draft_picks count")?;
        Ok(count > 0)
    }

    /// Delete all draft picks and draft state, resetting the draft to a clean
    /// slate. Player and projection data are preserved.
    pub fn clear_draft(&self) -> Result<()> {
        let conn = self.conn.lock().expect("database mutex poisoned");
        conn.execute_batch(
            "DELETE FROM draft_picks;
             DELETE FROM draft_state;",
        )
        .context("failed to clear draft data")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Helper: create a fresh in-memory database for each test.
    fn test_db() -> Database {
        Database::open(":memory:").expect("in-memory database should open")
    }

    /// Helper: build a sample DraftPick.
    fn sample_pick(pick_number: u32) -> DraftPick {
        DraftPick {
            pick_number,
            team_id: "team-1".to_string(),
            team_name: "Vorticists".to_string(),
            player_name: format!("Player {pick_number}"),
            position: "SS".to_string(),
            price: 25,
            timestamp: chrono::Utc::now(),
        }
    }

    // ------------------------------------------------------------------
    // Schema / open
    // ------------------------------------------------------------------

    #[test]
    fn open_creates_tables() {
        let db = test_db();
        let conn = db.conn.lock().unwrap();

        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(tables.contains(&"players".to_string()));
        assert!(tables.contains(&"projections".to_string()));
        assert!(tables.contains(&"draft_picks".to_string()));
        assert!(tables.contains(&"draft_state".to_string()));
    }

    // ------------------------------------------------------------------
    // Draft picks
    // ------------------------------------------------------------------

    #[test]
    fn insert_and_load_picks_round_trip() {
        let db = test_db();

        let pick1 = sample_pick(1);
        let pick2 = DraftPick {
            pick_number: 2,
            team_id: "team-2".to_string(),
            team_name: "Mudcats".to_string(),
            player_name: "Player 2".to_string(),
            position: "OF".to_string(),
            price: 40,
            timestamp: chrono::Utc::now(),
        };

        db.record_pick(&pick1).unwrap();
        db.record_pick(&pick2).unwrap();

        let picks = db.load_picks().unwrap();
        assert_eq!(picks.len(), 2);

        assert_eq!(picks[0].pick_number, 1);
        assert_eq!(picks[0].team_name, "Vorticists");
        assert_eq!(picks[0].player_name, "Player 1");
        assert_eq!(picks[0].price, 25);

        assert_eq!(picks[1].pick_number, 2);
        assert_eq!(picks[1].team_name, "Mudcats");
        assert_eq!(picks[1].player_name, "Player 2");
        assert_eq!(picks[1].price, 40);
    }

    #[test]
    fn load_picks_returns_empty_vec_when_no_picks() {
        let db = test_db();
        let picks = db.load_picks().unwrap();
        assert!(picks.is_empty());
    }

    // ------------------------------------------------------------------
    // Draft state (key-value)
    // ------------------------------------------------------------------

    #[test]
    fn save_and_load_state_round_trip() {
        let db = test_db();
        let value = json!({"round": 3, "nominations": ["A", "B"]});

        db.save_state("current_round", &value).unwrap();

        let loaded = db.load_state("current_round").unwrap();
        assert_eq!(loaded, Some(value));
    }

    #[test]
    fn load_state_returns_none_for_missing_key() {
        let db = test_db();
        let loaded = db.load_state("nonexistent").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn save_state_overwrites_previous_value() {
        let db = test_db();
        db.save_state("key", &json!(1)).unwrap();
        db.save_state("key", &json!(2)).unwrap();

        let loaded = db.load_state("key").unwrap();
        assert_eq!(loaded, Some(json!(2)));
    }

    // ------------------------------------------------------------------
    // has_draft_in_progress / clear_draft
    // ------------------------------------------------------------------

    #[test]
    fn has_draft_in_progress_false_then_true() {
        let db = test_db();
        assert!(!db.has_draft_in_progress().unwrap());

        db.record_pick(&sample_pick(1)).unwrap();
        assert!(db.has_draft_in_progress().unwrap());
    }

    #[test]
    fn clear_draft_resets_picks_and_state() {
        let db = test_db();

        db.record_pick(&sample_pick(1)).unwrap();
        db.save_state("budget", &json!(200)).unwrap();
        assert!(db.has_draft_in_progress().unwrap());

        db.clear_draft().unwrap();

        assert!(!db.has_draft_in_progress().unwrap());
        assert!(db.load_state("budget").unwrap().is_none());
    }

    // ------------------------------------------------------------------
    // Players
    // ------------------------------------------------------------------

    #[test]
    fn upsert_player_returns_id_and_no_duplicates() {
        let db = test_db();
        let positions = vec!["SS".to_string(), "2B".to_string()];

        let id1 = db
            .upsert_player("Trea Turner", "PHI", &positions, "batter")
            .unwrap();
        assert!(id1 > 0);

        // Upsert same player -> should return same id, not create a duplicate.
        let new_positions = vec!["SS".to_string()];
        let id2 = db
            .upsert_player("Trea Turner", "PHI", &new_positions, "batter")
            .unwrap();
        assert_eq!(id1, id2);

        // Verify only one row exists.
        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM players", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        // Verify the positions were updated.
        let stored_positions: String = conn
            .query_row(
                "SELECT positions FROM players WHERE id = ?1",
                params![id1],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_positions, r#"["SS"]"#);
    }

    #[test]
    fn upsert_player_different_teams_are_separate() {
        let db = test_db();
        let positions = vec!["OF".to_string()];

        let id1 = db
            .upsert_player("Juan Soto", "NYM", &positions, "batter")
            .unwrap();
        let id2 = db
            .upsert_player("Juan Soto", "NYY", &positions, "batter")
            .unwrap();

        assert_ne!(id1, id2);
    }

    // ------------------------------------------------------------------
    // Projections
    // ------------------------------------------------------------------

    #[test]
    fn insert_projection_works() {
        let db = test_db();
        let player_id = db
            .upsert_player("Shohei Ohtani", "LAD", &["DH".to_string()], "batter")
            .unwrap();

        db.insert_projection(player_id, "steamer", "HR", 45.0)
            .unwrap();
        db.insert_projection(player_id, "steamer", "RBI", 110.0)
            .unwrap();

        // Verify rows exist.
        let conn = db.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM projections WHERE player_id = ?1",
                params![player_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);

        // Verify values.
        let hr_val: f64 = conn
            .query_row(
                "SELECT value FROM projections WHERE player_id = ?1 AND stat_name = 'HR'",
                params![player_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!((hr_val - 45.0).abs() < f64::EPSILON);
    }

    #[test]
    fn insert_projection_replaces_on_conflict() {
        let db = test_db();
        let player_id = db
            .upsert_player("Aaron Judge", "NYY", &["OF".to_string()], "batter")
            .unwrap();

        db.insert_projection(player_id, "steamer", "HR", 50.0)
            .unwrap();
        db.insert_projection(player_id, "steamer", "HR", 55.0)
            .unwrap();

        let conn = db.conn.lock().unwrap();
        let val: f64 = conn
            .query_row(
                "SELECT value FROM projections WHERE player_id = ?1 AND source = 'steamer' AND stat_name = 'HR'",
                params![player_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!((val - 55.0).abs() < f64::EPSILON);
    }
}
