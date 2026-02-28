// SQLite persistence layer for draft state.

use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::draft::pick::DraftPick;

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

        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;",
        )
        .context("failed to set database pragmas")?;

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
                player_id INTEGER NOT NULL REFERENCES players(id),
                source    TEXT NOT NULL,
                stat_name TEXT NOT NULL,
                value     REAL NOT NULL,
                PRIMARY KEY (player_id, source, stat_name)
            );

            CREATE TABLE IF NOT EXISTS draft_picks (
                pick_number    INTEGER NOT NULL,
                team_id        TEXT NOT NULL,
                team_name      TEXT NOT NULL,
                player_id      INTEGER REFERENCES players(id),
                espn_player_id TEXT,
                player_name    TEXT NOT NULL,
                position       TEXT NOT NULL,
                price          INTEGER NOT NULL,
                eligible_slots TEXT,
                draft_id       TEXT NOT NULL DEFAULT '',
                timestamp      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                PRIMARY KEY (pick_number, draft_id)
            );

            CREATE TABLE IF NOT EXISTS draft_state (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            ",
        )
        .context("failed to create database schema")?;

        // Migration: add eligible_slots column if it doesn't exist (for pre-v0.2 databases)
        conn.execute_batch(
            "ALTER TABLE draft_picks ADD COLUMN eligible_slots TEXT;"
        ).ok(); // Silently ignore if column already exists (ALTER TABLE fails with "duplicate column name")

        // Migration: migrate draft_picks to composite primary key (pick_number, draft_id).
        // Detects old schema by checking if the draft_id column exists. If not, the
        // table is from a pre-draft-id version and needs a full table rebuild since
        // SQLite doesn't support altering primary keys in place.
        Self::migrate_draft_picks_add_draft_id(&conn)?;

        // Index on draft_id for efficient filtering. The composite PK is ordered
        // (pick_number, draft_id) so queries filtering by draft_id alone cannot
        // use it efficiently.
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_draft_picks_draft_id ON draft_picks(draft_id);"
        ).context("failed to create draft_id index")?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Migrate the `draft_picks` table from the old schema (pick_number as sole
    /// PRIMARY KEY) to the new schema with a composite PRIMARY KEY (pick_number,
    /// draft_id).
    ///
    /// This is a no-op if the table already has the `draft_id` column (new schema
    /// or previously migrated). For legacy databases, it rebuilds the table using
    /// SQLite's rename-create-copy-drop pattern since ALTER TABLE cannot change
    /// primary key constraints.
    fn migrate_draft_picks_add_draft_id(conn: &Connection) -> Result<()> {
        // Check if draft_id column already exists
        let has_draft_id: bool = conn
            .prepare("SELECT draft_id FROM draft_picks LIMIT 0")
            .is_ok();

        if has_draft_id {
            return Ok(()); // Already migrated or new schema
        }

        // Old schema detected: rebuild the table with composite primary key.
        // Legacy rows get draft_id = '' (empty string), making them invisible
        // to any real draft_id filter.
        conn.execute_batch(
            "
            ALTER TABLE draft_picks RENAME TO draft_picks_old;

            CREATE TABLE draft_picks (
                pick_number    INTEGER NOT NULL,
                team_id        TEXT NOT NULL,
                team_name      TEXT NOT NULL,
                player_id      INTEGER REFERENCES players(id),
                espn_player_id TEXT,
                player_name    TEXT NOT NULL,
                position       TEXT NOT NULL,
                price          INTEGER NOT NULL,
                eligible_slots TEXT,
                draft_id       TEXT NOT NULL DEFAULT '',
                timestamp      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                PRIMARY KEY (pick_number, draft_id)
            );

            INSERT INTO draft_picks (pick_number, team_id, team_name, player_id, espn_player_id, player_name, position, price, eligible_slots, draft_id, timestamp)
                SELECT pick_number, team_id, team_name, player_id, espn_player_id, player_name, position, price, eligible_slots, '', timestamp
                FROM draft_picks_old;

            DROP TABLE draft_picks_old;
            ",
        )
        .context("failed to migrate draft_picks table for draft_id support")?;

        Ok(())
    }

    /// Acquire the database connection.
    ///
    /// Panics if the mutex is poisoned (another thread panicked while
    /// holding the lock). This should never happen in normal operation.
    fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().expect("database mutex poisoned")
    }

    /// Record a single draft pick. Uses INSERT OR IGNORE for idempotency —
    /// re-recording the same pick_number is a no-op. Player linkage
    /// (`player_id`) is deferred as NULL. Timestamp is auto-generated by SQLite.
    ///
    /// The `draft_id` scopes this pick to a specific draft session so picks
    /// from different sessions don't intermingle.
    pub fn record_pick(&self, pick: &DraftPick, draft_id: &str) -> Result<()> {
        let conn = self.conn();
        let eligible_slots_json = serde_json::to_string(&pick.eligible_slots)
            .context("failed to serialize eligible_slots")?;
        conn.execute(
            "INSERT OR IGNORE INTO draft_picks
                (pick_number, team_id, team_name, espn_player_id, player_name, position, price, eligible_slots, draft_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                pick.pick_number,
                pick.team_id,
                pick.team_name,
                pick.espn_player_id,
                pick.player_name,
                pick.position,
                pick.price,
                eligible_slots_json,
                draft_id,
            ],
        )
        .context("failed to record draft pick")?;
        Ok(())
    }

    /// Load draft picks for a specific draft session, ordered by pick number.
    ///
    /// Only returns picks that match the given `draft_id`. Picks from other
    /// draft sessions (or legacy picks with empty draft_id) are excluded.
    pub fn load_picks(&self, draft_id: &str) -> Result<Vec<DraftPick>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT pick_number, team_id, team_name, player_name, position, price, espn_player_id, eligible_slots
                 FROM draft_picks WHERE draft_id = ?1 ORDER BY pick_number",
            )
            .context("failed to prepare load_picks query")?;

        let picks = stmt
            .query_map(params![draft_id], |row| {
                let eligible_slots_json: Option<String> = row.get(7)?;
                let eligible_slots = eligible_slots_json
                    .and_then(|json_str| serde_json::from_str::<Vec<u16>>(&json_str).ok())
                    .unwrap_or_default();
                Ok(DraftPick {
                    pick_number: row.get(0)?,
                    team_id: row.get(1)?,
                    team_name: row.get(2)?,
                    player_name: row.get(3)?,
                    position: row.get(4)?,
                    price: row.get(5)?,
                    espn_player_id: row.get(6)?,
                    eligible_slots,
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
        let conn = self.conn();
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
        let conn = self.conn();
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
    /// exists. Returns the player's row id in a single atomic statement.
    ///
    /// `positions` is stored as a JSON array string (e.g. `["SS","2B"]`).
    pub fn upsert_player(
        &self,
        name: &str,
        team: &str,
        positions: &[String],
        player_type: &str,
    ) -> Result<i64> {
        let conn = self.conn();
        let positions_json =
            serde_json::to_string(positions).context("failed to serialize positions")?;

        let id: i64 = conn
            .query_row(
                "INSERT INTO players (name, team, positions, player_type)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(name, team) DO UPDATE SET
                    positions   = excluded.positions,
                    player_type = excluded.player_type
                 RETURNING id",
                params![name, team, positions_json, player_type],
                |row| row.get(0),
            )
            .context("failed to upsert player")?;
        Ok(id)
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
        let conn = self.conn();
        conn.execute(
            "INSERT OR REPLACE INTO projections (player_id, source, stat_name, value)
             VALUES (?1, ?2, ?3, ?4)",
            params![player_id, source, stat_name, value],
        )
        .context("failed to insert projection")?;
        Ok(())
    }

    /// Returns `true` if at least one draft pick has been recorded for the
    /// given `draft_id`. Uses `SELECT EXISTS` for efficiency (stops after
    /// finding the first matching row rather than counting all rows).
    pub fn has_draft_in_progress(&self, draft_id: &str) -> Result<bool> {
        let conn = self.conn();
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM draft_picks WHERE draft_id = ?1)",
                params![draft_id],
                |row| row.get(0),
            )
            .context("failed to check draft_picks existence")?;
        Ok(exists)
    }

    /// Return the number of draft picks recorded for the given `draft_id`.
    pub fn pick_count(&self, draft_id: &str) -> Result<usize> {
        let conn = self.conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM draft_picks WHERE draft_id = ?1",
                params![draft_id],
                |row| row.get(0),
            )
            .context("failed to count draft picks")?;
        Ok(count as usize)
    }

    /// Delete all draft picks and draft state, resetting the draft to a clean
    /// slate. Player and projection data are preserved. Uses a proper
    /// transaction with automatic rollback on error.
    pub fn clear_draft(&self) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().context("failed to begin transaction")?;
        tx.execute("DELETE FROM draft_picks", [])
            .context("failed to delete draft picks")?;
        tx.execute("DELETE FROM draft_state", [])
            .context("failed to delete draft state")?;
        tx.commit().context("failed to commit clear_draft")?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Draft ID management
    // ------------------------------------------------------------------

    /// Key used in the draft_state table to store the current draft ID.
    const DRAFT_ID_KEY: &'static str = "current_draft_id";

    /// Retrieve the stored draft ID from the key-value store.
    /// Returns `None` if no draft ID has been set yet.
    pub fn get_draft_id(&self) -> Result<Option<String>> {
        let value = self.load_state(Self::DRAFT_ID_KEY)?;
        Ok(value.and_then(|v| v.as_str().map(|s| s.to_string())))
    }

    /// Persist a draft ID to the key-value store.
    pub fn set_draft_id(&self, draft_id: &str) -> Result<()> {
        self.save_state(
            Self::DRAFT_ID_KEY,
            &serde_json::Value::String(draft_id.to_string()),
        )
    }

    /// Generate a new unique draft ID based on the current UTC timestamp.
    ///
    /// Format: `draft_YYYYMMDD_HHMMSS_SSS` (e.g. `draft_20260228_143022_123`).
    /// The millisecond suffix ensures uniqueness even if two drafts start in
    /// the same second.
    pub fn generate_draft_id() -> String {
        let now = chrono::Utc::now();
        now.format("draft_%Y%m%d_%H%M%S_%3f").to_string()
    }

    /// Import players and projections in a single transaction.
    ///
    /// Each entry is a tuple of (name, team, positions, player_type, projections)
    /// where projections is a slice of (source, stat_name, value).
    pub fn import_players(
        &self,
        players: &[(&str, &str, &[String], &str, &[(&str, &str, f64)])],
    ) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction().context("failed to begin import transaction")?;

        for &(name, team, ref positions, player_type, ref projections) in players {
            let positions_json =
                serde_json::to_string(positions).context("failed to serialize positions")?;

            let player_id: i64 = tx
                .query_row(
                    "INSERT INTO players (name, team, positions, player_type)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(name, team) DO UPDATE SET
                        positions   = excluded.positions,
                        player_type = excluded.player_type
                     RETURNING id",
                    params![name, team, positions_json, player_type],
                    |row| row.get(0),
                )
                .context("failed to upsert player in batch")?;

            for &(source, stat_name, value) in *projections {
                tx.execute(
                    "INSERT OR REPLACE INTO projections (player_id, source, stat_name, value)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![player_id, source, stat_name, value],
                )
                .context("failed to insert projection in batch")?;
            }
        }

        tx.commit().context("failed to commit import")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Test draft ID used across all db tests.
    const TEST_DRAFT_ID: &str = "test_draft_001";

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
            espn_player_id: None,
            eligible_slots: vec![],
        }
    }

    // ------------------------------------------------------------------
    // Schema / open
    // ------------------------------------------------------------------

    #[test]
    fn open_creates_tables() {
        let db = test_db();
        let conn = db.conn();

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

        let pick1 = DraftPick {
            espn_player_id: Some("espn_1".to_string()),
            eligible_slots: vec![4, 2, 6, 12, 16, 17], // SS/2B/MI/UTIL/BE/IL
            ..sample_pick(1)
        };
        let pick2 = DraftPick {
            pick_number: 2,
            team_id: "team-2".to_string(),
            team_name: "Mudcats".to_string(),
            player_name: "Player 2".to_string(),
            position: "OF".to_string(),
            price: 40,
            espn_player_id: Some("espn_2".to_string()),
            eligible_slots: vec![],
        };

        db.record_pick(&pick1, TEST_DRAFT_ID).unwrap();
        db.record_pick(&pick2, TEST_DRAFT_ID).unwrap();

        let picks = db.load_picks(TEST_DRAFT_ID).unwrap();
        assert_eq!(picks.len(), 2);

        assert_eq!(picks[0].pick_number, 1);
        assert_eq!(picks[0].team_name, "Vorticists");
        assert_eq!(picks[0].player_name, "Player 1");
        assert_eq!(picks[0].price, 25);
        assert_eq!(picks[0].espn_player_id, Some("espn_1".to_string()));
        assert_eq!(picks[0].eligible_slots, vec![4, 2, 6, 12, 16, 17]);

        assert_eq!(picks[1].pick_number, 2);
        assert_eq!(picks[1].team_name, "Mudcats");
        assert_eq!(picks[1].player_name, "Player 2");
        assert_eq!(picks[1].price, 40);
        assert_eq!(picks[1].espn_player_id, Some("espn_2".to_string()));
        assert!(picks[1].eligible_slots.is_empty());
    }

    #[test]
    fn load_picks_returns_empty_vec_when_no_picks() {
        let db = test_db();
        let picks = db.load_picks(TEST_DRAFT_ID).unwrap();
        assert!(picks.is_empty());
    }

    #[test]
    fn record_pick_stores_espn_player_id() {
        let db = test_db();
        let pick_with = DraftPick {
            espn_player_id: Some("12345".to_string()),
            ..sample_pick(1)
        };
        db.record_pick(&pick_with, TEST_DRAFT_ID).unwrap();
        db.record_pick(&sample_pick(2), TEST_DRAFT_ID).unwrap();

        let picks = db.load_picks(TEST_DRAFT_ID).unwrap();
        assert_eq!(picks[0].espn_player_id, Some("12345".to_string()));
        assert_eq!(picks[1].espn_player_id, None);
    }

    #[test]
    fn record_pick_auto_generates_timestamp() {
        let db = test_db();
        db.record_pick(&sample_pick(1), TEST_DRAFT_ID).unwrap();

        let conn = db.conn();
        let ts: String = conn
            .query_row(
                "SELECT timestamp FROM draft_picks WHERE pick_number = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        // Should be a non-empty ISO-8601-ish string
        assert!(!ts.is_empty());
        assert!(ts.contains('T'));
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
        assert!(!db.has_draft_in_progress(TEST_DRAFT_ID).unwrap());

        db.record_pick(&sample_pick(1), TEST_DRAFT_ID).unwrap();
        assert!(db.has_draft_in_progress(TEST_DRAFT_ID).unwrap());
    }

    #[test]
    fn clear_draft_resets_picks_and_state() {
        let db = test_db();

        db.record_pick(&sample_pick(1), TEST_DRAFT_ID).unwrap();
        db.save_state("budget", &json!(200)).unwrap();
        assert!(db.has_draft_in_progress(TEST_DRAFT_ID).unwrap());

        db.clear_draft().unwrap();

        assert!(!db.has_draft_in_progress(TEST_DRAFT_ID).unwrap());
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
        let conn = db.conn();
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
        let conn = db.conn();
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

        let conn = db.conn();
        let val: f64 = conn
            .query_row(
                "SELECT value FROM projections WHERE player_id = ?1 AND source = 'steamer' AND stat_name = 'HR'",
                params![player_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!((val - 55.0).abs() < f64::EPSILON);
    }

    // ------------------------------------------------------------------
    // Idempotent record_pick
    // ------------------------------------------------------------------

    #[test]
    fn record_pick_idempotent_on_duplicate() {
        let db = test_db();
        db.record_pick(&sample_pick(1), TEST_DRAFT_ID).unwrap();
        // Recording the same pick_number again should be a no-op, not an error.
        db.record_pick(&sample_pick(1), TEST_DRAFT_ID).unwrap();

        let picks = db.load_picks(TEST_DRAFT_ID).unwrap();
        assert_eq!(picks.len(), 1);
    }

    // ------------------------------------------------------------------
    // load_picks includes espn_player_id
    // ------------------------------------------------------------------

    #[test]
    fn load_picks_includes_espn_player_id() {
        let db = test_db();
        let pick = DraftPick {
            espn_player_id: Some("espn_42".to_string()),
            ..sample_pick(1)
        };
        db.record_pick(&pick, TEST_DRAFT_ID).unwrap();
        db.record_pick(&sample_pick(2), TEST_DRAFT_ID).unwrap();

        let picks = db.load_picks(TEST_DRAFT_ID).unwrap();
        assert_eq!(picks[0].espn_player_id, Some("espn_42".to_string()));
        assert_eq!(picks[1].espn_player_id, None);
    }

    // ------------------------------------------------------------------
    // eligible_slots persistence
    // ------------------------------------------------------------------

    #[test]
    fn load_picks_persists_eligible_slots() {
        let db = test_db();
        let pick = DraftPick {
            eligible_slots: vec![4, 2, 6, 12, 16, 17], // SS/2B/MI/UTIL/BE/IL
            ..sample_pick(1)
        };
        db.record_pick(&pick, TEST_DRAFT_ID).unwrap();

        let picks = db.load_picks(TEST_DRAFT_ID).unwrap();
        assert_eq!(picks[0].eligible_slots, vec![4, 2, 6, 12, 16, 17]);
    }

    #[test]
    fn load_picks_empty_eligible_slots() {
        let db = test_db();
        db.record_pick(&sample_pick(1), TEST_DRAFT_ID).unwrap();

        let picks = db.load_picks(TEST_DRAFT_ID).unwrap();
        assert!(picks[0].eligible_slots.is_empty());
    }

    // ------------------------------------------------------------------
    // Foreign keys enforced
    // ------------------------------------------------------------------

    #[test]
    fn foreign_keys_enforced() {
        let db = test_db();
        // Inserting a projection with a non-existent player_id should fail
        // because foreign_keys = ON.
        let result = db.insert_projection(9999, "steamer", "HR", 30.0);
        assert!(result.is_err());
    }

    // ------------------------------------------------------------------
    // Batch import
    // ------------------------------------------------------------------

    #[test]
    fn import_players_batch() {
        let db = test_db();
        let positions1 = vec!["SS".to_string(), "2B".to_string()];
        let positions2 = vec!["OF".to_string()];

        let players: Vec<(&str, &str, &[String], &str, &[(&str, &str, f64)])> = vec![
            (
                "Trea Turner",
                "PHI",
                &positions1,
                "batter",
                &[("steamer", "HR", 20.0), ("steamer", "SB", 30.0)],
            ),
            (
                "Aaron Judge",
                "NYY",
                &positions2,
                "batter",
                &[("steamer", "HR", 55.0)],
            ),
        ];

        db.import_players(&players).unwrap();

        // Verify players
        let conn = db.conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM players", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);

        // Verify projections
        let proj_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM projections", [], |row| row.get(0))
            .unwrap();
        assert_eq!(proj_count, 3);
    }

    // ------------------------------------------------------------------
    // player_id column allows NULL in draft_picks
    // ------------------------------------------------------------------

    #[test]
    fn draft_pick_player_id_is_null() {
        let db = test_db();
        db.record_pick(&sample_pick(1), TEST_DRAFT_ID).unwrap();

        let conn = db.conn();
        let player_id: Option<i64> = conn
            .query_row(
                "SELECT player_id FROM draft_picks WHERE pick_number = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(player_id.is_none());
    }

    // ------------------------------------------------------------------
    // Schema migration: eligible_slots column added to pre-existing table
    // ------------------------------------------------------------------

    #[test]
    fn migration_adds_eligible_slots_to_existing_table() {
        // Simulate a pre-v0.2 database that lacks the eligible_slots column.
        let conn = Connection::open(":memory:").unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;",
        )
        .unwrap();
        conn.execute_batch(
            "CREATE TABLE players (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT NOT NULL,
                team        TEXT NOT NULL,
                positions   TEXT NOT NULL,
                player_type TEXT NOT NULL,
                UNIQUE(name, team)
            );
            CREATE TABLE projections (
                player_id INTEGER NOT NULL REFERENCES players(id),
                source    TEXT NOT NULL,
                stat_name TEXT NOT NULL,
                value     REAL NOT NULL,
                PRIMARY KEY (player_id, source, stat_name)
            );
            CREATE TABLE draft_picks (
                pick_number    INTEGER PRIMARY KEY,
                team_id        TEXT NOT NULL,
                team_name      TEXT NOT NULL,
                player_name    TEXT NOT NULL,
                position       TEXT NOT NULL,
                price          INTEGER NOT NULL,
                player_id      INTEGER REFERENCES players(id),
                espn_player_id TEXT,
                timestamp      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            );
            CREATE TABLE draft_state (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )
        .unwrap();

        // Insert a legacy pick (no eligible_slots column yet)
        conn.execute(
            "INSERT INTO draft_picks (pick_number, team_id, team_name, player_name, position, price)
             VALUES (1, 'team-1', 'Vorticists', 'Legacy Player', 'SS', 20)",
            [],
        )
        .unwrap();
        drop(conn);

        // Now use a temp file so Database::open can re-open the same DB.
        // We use an in-memory approach by opening the raw connection, attaching
        // the legacy schema, then letting Database::open do the migration.
        // Actually, let's use a temp file for this test.
        let tmp_dir = std::env::temp_dir();
        let db_path = tmp_dir.join(format!("test_migration_{}.db", std::process::id()));
        let db_path_str = db_path.to_str().unwrap();

        // Create legacy database on disk
        {
            let conn = Connection::open(db_path_str).unwrap();
            conn.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA busy_timeout = 5000;
                 PRAGMA foreign_keys = ON;",
            )
            .unwrap();
            conn.execute_batch(
                "CREATE TABLE players (
                    id          INTEGER PRIMARY KEY AUTOINCREMENT,
                    name        TEXT NOT NULL,
                    team        TEXT NOT NULL,
                    positions   TEXT NOT NULL,
                    player_type TEXT NOT NULL,
                    UNIQUE(name, team)
                );
                CREATE TABLE projections (
                    player_id INTEGER NOT NULL REFERENCES players(id),
                    source    TEXT NOT NULL,
                    stat_name TEXT NOT NULL,
                    value     REAL NOT NULL,
                    PRIMARY KEY (player_id, source, stat_name)
                );
                CREATE TABLE draft_picks (
                    pick_number    INTEGER PRIMARY KEY,
                    team_id        TEXT NOT NULL,
                    team_name      TEXT NOT NULL,
                    player_name    TEXT NOT NULL,
                    position       TEXT NOT NULL,
                    price          INTEGER NOT NULL,
                    player_id      INTEGER REFERENCES players(id),
                    espn_player_id TEXT,
                    timestamp      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                );
                CREATE TABLE draft_state (
                    key   TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO draft_picks (pick_number, team_id, team_name, player_name, position, price)
                 VALUES (1, 'team-1', 'Vorticists', 'Legacy Player', 'SS', 20)",
                [],
            )
            .unwrap();
        }

        // Open with Database::open — migration should add eligible_slots and draft_id columns
        let db = Database::open(db_path_str).expect("migration should succeed");

        // Legacy picks have NULL draft_id, so they should NOT appear
        // when loading with a specific draft_id.
        let picks = db.load_picks(TEST_DRAFT_ID).unwrap();
        assert_eq!(picks.len(), 0, "Legacy picks with NULL draft_id should not be loaded");

        // Legacy picks should not count for has_draft_in_progress either
        assert!(!db.has_draft_in_progress(TEST_DRAFT_ID).unwrap());

        // Verify new picks with eligible_slots and draft_id can be recorded
        let new_pick = DraftPick {
            pick_number: 2,
            team_id: "team-2".to_string(),
            team_name: "Mudcats".to_string(),
            player_name: "New Player".to_string(),
            position: "CF".to_string(),
            price: 35,
            espn_player_id: Some("espn_99".to_string()),
            eligible_slots: vec![9, 5, 12, 16, 17],
        };
        db.record_pick(&new_pick, TEST_DRAFT_ID).unwrap();

        let picks = db.load_picks(TEST_DRAFT_ID).unwrap();
        assert_eq!(picks.len(), 1);
        assert_eq!(picks[0].player_name, "New Player");
        assert_eq!(picks[0].eligible_slots, vec![9, 5, 12, 16, 17]);

        // Clean up temp file
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(format!("{}-wal", db_path_str));
        let _ = std::fs::remove_file(format!("{}-shm", db_path_str));
    }

    // ------------------------------------------------------------------
    // Draft ID isolation
    // ------------------------------------------------------------------

    #[test]
    fn picks_scoped_to_draft_id() {
        let db = test_db();
        let draft_a = "draft_a";
        let draft_b = "draft_b";

        // Record picks in two different drafts
        db.record_pick(&sample_pick(1), draft_a).unwrap();
        db.record_pick(&sample_pick(2), draft_a).unwrap();
        db.record_pick(&sample_pick(3), draft_b).unwrap();

        // Each draft should only see its own picks
        let picks_a = db.load_picks(draft_a).unwrap();
        assert_eq!(picks_a.len(), 2);
        assert_eq!(picks_a[0].pick_number, 1);
        assert_eq!(picks_a[1].pick_number, 2);

        let picks_b = db.load_picks(draft_b).unwrap();
        assert_eq!(picks_b.len(), 1);
        assert_eq!(picks_b[0].pick_number, 3);

        // has_draft_in_progress should be scoped too
        assert!(db.has_draft_in_progress(draft_a).unwrap());
        assert!(db.has_draft_in_progress(draft_b).unwrap());
        assert!(!db.has_draft_in_progress("draft_nonexistent").unwrap());
    }

    #[test]
    fn draft_id_persists_via_state_store() {
        let db = test_db();

        // Initially no draft ID stored
        assert!(db.get_draft_id().unwrap().is_none());

        // Store a draft ID
        db.set_draft_id("draft_20260228_143022_123").unwrap();
        assert_eq!(
            db.get_draft_id().unwrap(),
            Some("draft_20260228_143022_123".to_string())
        );

        // Overwrite with a new draft ID
        db.set_draft_id("draft_20260301_090000_456").unwrap();
        assert_eq!(
            db.get_draft_id().unwrap(),
            Some("draft_20260301_090000_456".to_string())
        );
    }

    #[test]
    fn generate_draft_id_format() {
        let id = Database::generate_draft_id();
        assert!(id.starts_with("draft_"), "Draft ID should start with 'draft_': {}", id);
        // Should be ~25 chars: draft_YYYYMMDD_HHMMSS_SSS
        assert!(id.len() >= 24, "Draft ID should be at least 24 chars: {}", id);
    }

    #[test]
    fn old_draft_picks_invisible_to_new_draft() {
        let db = test_db();
        let old_draft = "draft_old";
        let new_draft = "draft_new";

        // Simulate a completed old draft with many picks
        for i in 1..=10 {
            db.record_pick(&sample_pick(i), old_draft).unwrap();
        }
        assert_eq!(db.load_picks(old_draft).unwrap().len(), 10);

        // New draft should start empty
        assert!(!db.has_draft_in_progress(new_draft).unwrap());
        assert!(db.load_picks(new_draft).unwrap().is_empty());

        // Recording a pick in the new draft should not affect the old
        db.record_pick(&sample_pick(1), new_draft).unwrap();
        assert_eq!(db.load_picks(old_draft).unwrap().len(), 10);
        assert_eq!(db.load_picks(new_draft).unwrap().len(), 1);
    }

    #[test]
    fn clear_draft_removes_all_drafts() {
        let db = test_db();

        db.record_pick(&sample_pick(1), "draft_a").unwrap();
        db.record_pick(&sample_pick(2), "draft_b").unwrap();
        db.set_draft_id("draft_b").unwrap();

        db.clear_draft().unwrap();

        // All picks from all drafts should be gone
        assert!(!db.has_draft_in_progress("draft_a").unwrap());
        assert!(!db.has_draft_in_progress("draft_b").unwrap());
        // Draft state (including stored draft_id) should be cleared
        assert!(db.get_draft_id().unwrap().is_none());
    }
}
