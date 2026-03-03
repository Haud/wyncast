// Structured database migration system for the draft-assistant SQLite database.
//
// Migrations are versioned SQL scripts stored as static data. The
// `schema_migrations` table tracks which migrations have been applied.
// New databases run all migrations from v1. Legacy databases that were
// already fully set up before this system was introduced get bootstrapped
// (all four historical migrations marked applied without re-running).

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Tracking table DDL. Created once on first open; idempotent.
const CREATE_MIGRATIONS_TABLE: &str = "
    CREATE TABLE IF NOT EXISTS schema_migrations (
        version     INTEGER PRIMARY KEY,
        name        TEXT NOT NULL,
        applied_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    );
";

/// A single versioned migration step.
pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    /// SQL executed when applying this migration.
    pub up: &'static str,
    /// SQL executed when rolling back this migration. `None` means irreversible.
    pub down: Option<&'static str>,
}

/// All known migrations, in ascending version order.
///
/// Migration 1's `up` creates the *final* schema including every column added
/// by migrations 2-4 so that fresh databases reach the current state in a
/// single step. Migrations 2-4 are incremental steps for old databases that
/// were set up before this migration system existed; new databases skip them
/// via the bootstrap logic in `ensure_migrations_table`.
static MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial_schema",
        up: "
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
                assigned_slot  INTEGER,
                draft_id       TEXT NOT NULL DEFAULT '',
                timestamp      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                PRIMARY KEY (pick_number, draft_id)
            );

            CREATE TABLE IF NOT EXISTS draft_state (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_draft_picks_draft_id ON draft_picks(draft_id);
        ",
        down: Some(
            "DROP TABLE draft_state;
             DROP TABLE draft_picks;
             DROP TABLE projections;
             DROP TABLE players;",
        ),
    },
    Migration {
        version: 2,
        name: "add_eligible_slots",
        up: "ALTER TABLE draft_picks ADD COLUMN eligible_slots TEXT;",
        down: Some("ALTER TABLE draft_picks DROP COLUMN eligible_slots;"),
    },
    Migration {
        version: 3,
        name: "add_draft_id_composite_pk",
        up: "
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

            INSERT INTO draft_picks
                (pick_number, team_id, team_name, player_id, espn_player_id,
                 player_name, position, price, eligible_slots, draft_id, timestamp)
            SELECT
                pick_number, team_id, team_name, player_id, espn_player_id,
                player_name, position, price, eligible_slots, '', timestamp
            FROM draft_picks_old;

            DROP TABLE draft_picks_old;
        ",
        down: Some("
            ALTER TABLE draft_picks RENAME TO draft_picks_old;

            CREATE TABLE draft_picks (
                pick_number    INTEGER PRIMARY KEY,
                team_id        TEXT NOT NULL,
                team_name      TEXT NOT NULL,
                player_id      INTEGER REFERENCES players(id),
                espn_player_id TEXT,
                player_name    TEXT NOT NULL,
                position       TEXT NOT NULL,
                price          INTEGER NOT NULL,
                eligible_slots TEXT,
                timestamp      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            );

            INSERT INTO draft_picks
                (pick_number, team_id, team_name, player_id, espn_player_id,
                 player_name, position, price, eligible_slots, timestamp)
            SELECT
                pick_number, team_id, team_name, player_id, espn_player_id,
                player_name, position, price, eligible_slots, timestamp
            FROM draft_picks_old;

            DROP TABLE draft_picks_old;
        "),
    },
    Migration {
        version: 4,
        name: "add_assigned_slot",
        up: "ALTER TABLE draft_picks ADD COLUMN assigned_slot INTEGER;",
        down: Some("ALTER TABLE draft_picks DROP COLUMN assigned_slot;"),
    },
];

/// Drives schema migrations for the SQLite database.
pub struct MigrationRunner;

impl MigrationRunner {
    /// Apply all pending migrations in ascending version order.
    ///
    /// Each migration is executed inside its own transaction so a failure
    /// leaves the database at the last successfully applied version.
    ///
    /// Special case: migration 1 creates the *final* schema including all
    /// columns added by migrations 2-4. When migration 1 is applied to a fresh
    /// database, migrations 2-4 are immediately marked as applied (without
    /// executing their SQL) because their changes are already present in the
    /// schema created by migration 1. This is done inside migration 1's
    /// transaction so that subsequent iterations of the loop see them as applied
    /// and skip them.
    pub fn run_pending(conn: &Connection) -> Result<()> {
        Self::ensure_migrations_table(conn)?;
        // Iterate over all known migrations in order, checking at each step
        // whether the migration is still pending. This allows the migration 1
        // bootstrap (which marks 2-4 applied) to take effect during the same
        // run without re-executing those migrations.
        let all_versions: Vec<i64> = MIGRATIONS.iter().map(|m| m.version).collect();
        for &version in &all_versions {
            // Re-check current version at each step so we see any bootstrap
            // inserts done in a previous iteration.
            let current = Self::current_version(conn)?;
            if version <= current {
                continue;
            }
            let migration = MIGRATIONS
                .iter()
                .find(|m| m.version == version)
                .expect("version must be in MIGRATIONS");

            conn.execute_batch("BEGIN;")
                .with_context(|| format!("failed to begin transaction for migration v{}", migration.version))?;
            let result = conn
                .execute_batch(migration.up)
                .with_context(|| format!("failed to apply migration v{} '{}'", migration.version, migration.name));
            if let Err(e) = result {
                let _ = conn.execute_batch("ROLLBACK;");
                return Err(e);
            }
            let insert_result = conn
                .execute(
                    "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                    rusqlite::params![migration.version, migration.name],
                )
                .with_context(|| format!("failed to record migration v{}", migration.version));
            if let Err(e) = insert_result {
                let _ = conn.execute_batch("ROLLBACK;");
                return Err(e);
            }
            // Migration 1 creates the final schema which already includes all
            // columns added by migrations 2-4. Mark them applied immediately so
            // that subsequent iterations of this loop skip them.
            if migration.version == 1 {
                for later in MIGRATIONS.iter().filter(|m| m.version > 1) {
                    let r = conn.execute(
                        "INSERT OR IGNORE INTO schema_migrations (version, name) VALUES (?1, ?2)",
                        rusqlite::params![later.version, later.name],
                    )
                    .with_context(|| format!("failed to bootstrap migration record for v{}", later.version));
                    if let Err(e) = r {
                        let _ = conn.execute_batch("ROLLBACK;");
                        return Err(e);
                    }
                }
            }
            conn.execute_batch("COMMIT;")
                .with_context(|| format!("failed to commit migration v{}", migration.version))?;
        }
        Ok(())
    }

    /// Roll back all migrations with version > `target_version`, in descending
    /// order.
    ///
    /// Returns an error if any migration in range has `down = None`
    /// (irreversible).
    pub fn rollback_to(conn: &Connection, target_version: i64) -> Result<()> {
        Self::ensure_migrations_table(conn)?;
        let current = Self::current_version(conn)?;

        // Collect applied migrations above the target, descending.
        let mut to_rollback: Vec<&'static Migration> = MIGRATIONS
            .iter()
            .filter(|m| m.version > target_version && m.version <= current)
            .collect();
        to_rollback.sort_by(|a, b| b.version.cmp(&a.version));

        for migration in to_rollback {
            let down_sql = migration.down.ok_or_else(|| {
                anyhow::anyhow!(
                    "migration v{} '{}' is irreversible (no down SQL)",
                    migration.version,
                    migration.name
                )
            })?;

            conn.execute_batch("BEGIN;")
                .with_context(|| format!("failed to begin rollback transaction for migration v{}", migration.version))?;
            let result = conn
                .execute_batch(down_sql)
                .with_context(|| format!("failed to roll back migration v{} '{}'", migration.version, migration.name));
            if let Err(e) = result {
                let _ = conn.execute_batch("ROLLBACK;");
                return Err(e);
            }
            let delete_result = conn
                .execute(
                    "DELETE FROM schema_migrations WHERE version = ?1",
                    rusqlite::params![migration.version],
                )
                .with_context(|| format!("failed to remove migration record for v{}", migration.version));
            if let Err(e) = delete_result {
                let _ = conn.execute_batch("ROLLBACK;");
                return Err(e);
            }
            conn.execute_batch("COMMIT;")
                .with_context(|| format!("failed to commit rollback of migration v{}", migration.version))?;
        }
        Ok(())
    }

    /// Return the highest applied migration version, or 0 if none have been
    /// applied.
    pub fn current_version(conn: &Connection) -> Result<i64> {
        conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .context("failed to query current migration version")
    }

    /// Create the `schema_migrations` tracking table if it does not exist, and
    /// bootstrap legacy databases.
    ///
    /// Bootstrap logic:
    /// - If `schema_migrations` is empty **and** the `players` table already
    ///   exists **and** `draft_picks` already has the `assigned_slot` column,
    ///   the database was fully migrated before this migration system was
    ///   introduced. Mark all four historical migrations as applied so that
    ///   `run_pending` becomes a no-op and the incremental migrations (2-4) are
    ///   not re-executed on the existing schema.
    /// - If `schema_migrations` is empty **and** `players` exists but the schema
    ///   is not fully migrated, the database is a partially-migrated legacy DB.
    ///   Do nothing and let `run_pending` apply the missing incremental steps.
    /// - If `schema_migrations` is empty **and** `players` does not exist, this
    ///   is a brand-new database. Do nothing and let `run_pending` apply all
    ///   migrations from v1.
    fn ensure_migrations_table(conn: &Connection) -> Result<()> {
        conn.execute_batch(CREATE_MIGRATIONS_TABLE)
            .context("failed to create schema_migrations table")?;

        // Check whether any migrations have already been recorded.
        let recorded_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .context("failed to count schema_migrations rows")?;

        if recorded_count > 0 {
            // Migrations table is populated; nothing to bootstrap.
            return Ok(());
        }

        // Check whether the players table exists (legacy database indicator).
        let players_exists: bool = conn
            .query_row(
                "SELECT name FROM sqlite_master WHERE type='table' AND name='players'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);

        if !players_exists {
            // Fresh database: do nothing; run_pending will apply all migrations.
            return Ok(());
        }

        // Players table exists — this is a legacy database. Mark migration 1 as
        // applied (the tables already exist; re-running CREATE TABLE IF NOT EXISTS
        // would be a no-op but we avoid the confusion). Migrations 2-4 will be
        // applied only if the corresponding columns are still missing.
        //
        // Also check whether the database was already fully migrated (all
        // incremental columns present). If so, mark 2-4 applied as well.
        let now = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

        // Always bootstrap migration 1: the base tables exist.
        if let Some(m1) = MIGRATIONS.iter().find(|m| m.version == 1) {
            conn.execute(
                &format!(
                    "INSERT OR IGNORE INTO schema_migrations (version, name, applied_at) \
                     VALUES (?1, ?2, {now})"
                ),
                rusqlite::params![m1.version, m1.name],
            )
            .context("failed to bootstrap migration record for v1")?;
        }

        // Migration 2: eligible_slots column. Mark applied if already present.
        let has_eligible_slots: bool = conn
            .prepare("SELECT eligible_slots FROM draft_picks LIMIT 0")
            .is_ok();
        if has_eligible_slots {
            if let Some(m) = MIGRATIONS.iter().find(|m| m.version == 2) {
                conn.execute(
                    &format!(
                        "INSERT OR IGNORE INTO schema_migrations (version, name, applied_at) \
                         VALUES (?1, ?2, {now})"
                    ),
                    rusqlite::params![m.version, m.name],
                )
                .context("failed to bootstrap migration record for v2")?;
            }
        }

        // Migration 3: composite PK (draft_id column). Mark applied if already present.
        let has_draft_id: bool = conn
            .prepare("SELECT draft_id FROM draft_picks LIMIT 0")
            .is_ok();
        if has_draft_id {
            if let Some(m) = MIGRATIONS.iter().find(|m| m.version == 3) {
                conn.execute(
                    &format!(
                        "INSERT OR IGNORE INTO schema_migrations (version, name, applied_at) \
                         VALUES (?1, ?2, {now})"
                    ),
                    rusqlite::params![m.version, m.name],
                )
                .context("failed to bootstrap migration record for v3")?;
            }
        }

        // Migration 4: assigned_slot column. Mark applied if already present.
        let has_assigned_slot: bool = conn
            .prepare("SELECT assigned_slot FROM draft_picks LIMIT 0")
            .is_ok();
        if has_assigned_slot {
            if let Some(m) = MIGRATIONS.iter().find(|m| m.version == 4) {
                conn.execute(
                    &format!(
                        "INSERT OR IGNORE INTO schema_migrations (version, name, applied_at) \
                         VALUES (?1, ?2, {now})"
                    ),
                    rusqlite::params![m.version, m.name],
                )
                .context("failed to bootstrap migration record for v4")?;
            }
        }

        Ok(())
    }

    /// Return migrations that have not yet been applied, sorted ascending by
    /// version.
    fn pending(conn: &Connection) -> Result<Vec<&'static Migration>> {
        let current = Self::current_version(conn)?;
        let mut pending: Vec<&'static Migration> =
            MIGRATIONS.iter().filter(|m| m.version > current).collect();
        pending.sort_by_key(|m| m.version);
        Ok(pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_memory() -> Connection {
        Connection::open(":memory:").expect("in-memory db")
    }

    #[test]
    fn fresh_db_runs_all_migrations() {
        let conn = in_memory();
        MigrationRunner::run_pending(&conn).expect("run_pending");
        assert_eq!(MigrationRunner::current_version(&conn).unwrap(), 4);
    }

    #[test]
    fn idempotent_run() {
        let conn = in_memory();
        MigrationRunner::run_pending(&conn).expect("first run");
        MigrationRunner::run_pending(&conn).expect("second run");
        assert_eq!(MigrationRunner::current_version(&conn).unwrap(), 4);
    }

    #[test]
    fn legacy_bootstrap_skips_incremental_migrations() {
        let conn = in_memory();

        // Simulate a legacy database: create the final schema directly without
        // going through the migration system.
        conn.execute_batch("
            CREATE TABLE players (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL, team TEXT NOT NULL,
                positions TEXT NOT NULL, player_type TEXT NOT NULL,
                UNIQUE(name, team)
            );
            CREATE TABLE projections (
                player_id INTEGER NOT NULL REFERENCES players(id),
                source TEXT NOT NULL, stat_name TEXT NOT NULL, value REAL NOT NULL,
                PRIMARY KEY (player_id, source, stat_name)
            );
            CREATE TABLE draft_picks (
                pick_number INTEGER NOT NULL, team_id TEXT NOT NULL,
                team_name TEXT NOT NULL, player_id INTEGER REFERENCES players(id),
                espn_player_id TEXT, player_name TEXT NOT NULL,
                position TEXT NOT NULL, price INTEGER NOT NULL,
                eligible_slots TEXT, assigned_slot INTEGER,
                draft_id TEXT NOT NULL DEFAULT '',
                timestamp TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                PRIMARY KEY (pick_number, draft_id)
            );
            CREATE TABLE draft_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS idx_draft_picks_draft_id ON draft_picks(draft_id);
        ").expect("create legacy schema");

        // Running migrations should bootstrap and then be a no-op.
        MigrationRunner::run_pending(&conn).expect("run_pending on legacy db");
        assert_eq!(MigrationRunner::current_version(&conn).unwrap(), 4);

        // The draft_picks table must still exist and have the expected columns.
        conn.execute_batch(
            "INSERT INTO draft_picks (pick_number, team_id, team_name, player_name, position, price, draft_id) \
             VALUES (1, 't1', 'Team 1', 'Player A', 'SP', 10, 'draft-1')"
        ).expect("insert into bootstrapped legacy table");
    }

    #[test]
    fn rollback_to_removes_migrations() {
        let conn = in_memory();
        MigrationRunner::run_pending(&conn).expect("run_pending");
        assert_eq!(MigrationRunner::current_version(&conn).unwrap(), 4);

        // Roll back to v1 (removes v4 assigned_slot column, v3 composite PK rebuild,
        // v2 eligible_slots column).
        MigrationRunner::rollback_to(&conn, 1).expect("rollback_to v1");
        assert_eq!(MigrationRunner::current_version(&conn).unwrap(), 1);
    }
}
