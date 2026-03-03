// Structured database migration system.
//
// Migrations are versioned SQL files embedded at compile time. The
// `schema_migrations` table tracks which have been applied. Each migration
// runs in its own transaction; a failure leaves the database at the last
// successfully applied version.

use anyhow::{Context, Result};
use rusqlite::Connection;

/// A single versioned migration step.
pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    /// SQL to apply this migration.
    pub up: &'static str,
    /// SQL to roll back this migration. `None` means irreversible.
    pub down: Option<&'static str>,
}

/// All known migrations, in ascending version order.
static MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "initial_schema",
    up: include_str!("../migrations/V001__initial_schema.up.sql"),
    down: Some(include_str!("../migrations/V001__initial_schema.down.sql")),
}];

/// Drives schema migrations for the SQLite database.
pub struct MigrationRunner;

impl MigrationRunner {
    /// Apply all pending migrations in ascending version order.
    ///
    /// Creates `schema_migrations` if it does not exist, then applies any
    /// migration not yet recorded, each in its own transaction.
    pub fn run_pending(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version     INTEGER PRIMARY KEY,
                name        TEXT NOT NULL,
                applied_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            );",
        )
        .context("failed to create schema_migrations table")?;

        for migration in MIGRATIONS {
            let applied: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
                    rusqlite::params![migration.version],
                    |row| row.get(0),
                )
                .with_context(|| {
                    format!("failed to check status of migration v{}", migration.version)
                })?;

            if applied {
                continue;
            }

            // `unchecked_transaction` is used because `run_pending` takes `&Connection`
            // (required since it is called before the connection is moved into Mutex).
            // No other transaction is active at this call site.
            let tx = conn.unchecked_transaction().with_context(|| {
                format!(
                    "failed to begin transaction for migration v{}",
                    migration.version
                )
            })?;
            tx.execute_batch(migration.up).with_context(|| {
                format!(
                    "failed to apply migration v{} '{}'",
                    migration.version, migration.name
                )
            })?;
            tx.execute(
                "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                rusqlite::params![migration.version, migration.name],
            )
            .with_context(|| format!("failed to record migration v{}", migration.version))?;
            tx.commit().with_context(|| {
                format!("failed to commit migration v{}", migration.version)
            })?;
        }
        Ok(())
    }

    /// Roll back all applied migrations with version > `target_version`, in
    /// descending order.
    ///
    /// Returns an error if any migration to be rolled back has `down = None`.
    /// Silently skips migrations in `MIGRATIONS` that are not recorded as applied
    /// in `schema_migrations`. If no migrations qualify, returns `Ok(())`.
    ///
    /// Requires `run_pending` to have been called at least once on this connection
    /// so that `schema_migrations` exists; errors with "no such table" otherwise.
    pub fn rollback_to(conn: &Connection, target_version: i64) -> Result<()> {
        let mut to_rollback: Vec<&'static Migration> = MIGRATIONS
            .iter()
            .filter(|m| m.version > target_version)
            .collect();
        to_rollback.sort_by(|a, b| b.version.cmp(&a.version));

        for migration in to_rollback {
            let applied: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
                    rusqlite::params![migration.version],
                    |row| row.get(0),
                )
                .with_context(|| {
                    format!(
                        "failed to check status of migration v{}",
                        migration.version
                    )
                })?;

            if !applied {
                continue;
            }

            let down_sql = migration.down.ok_or_else(|| {
                anyhow::anyhow!(
                    "migration v{} '{}' is irreversible (no down SQL)",
                    migration.version,
                    migration.name
                )
            })?;

            // `unchecked_transaction` is used because `rollback_to` takes `&Connection`
            // (required since it is called before the connection is moved into Mutex).
            // No other transaction is active at this call site.
            let tx = conn.unchecked_transaction().with_context(|| {
                format!(
                    "failed to begin rollback transaction for migration v{}",
                    migration.version
                )
            })?;
            tx.execute_batch(down_sql).with_context(|| {
                format!(
                    "failed to roll back migration v{} '{}'",
                    migration.version, migration.name
                )
            })?;
            tx.execute(
                "DELETE FROM schema_migrations WHERE version = ?1",
                rusqlite::params![migration.version],
            )
            .with_context(|| {
                format!(
                    "failed to remove migration record for v{}",
                    migration.version
                )
            })?;
            tx.commit().with_context(|| {
                format!(
                    "failed to commit rollback of migration v{}",
                    migration.version
                )
            })?;
        }
        Ok(())
    }

    /// Return the highest applied migration version, or 0 if none have been
    /// applied. Requires `schema_migrations` to exist; call `run_pending` first.
    pub fn current_version(conn: &Connection) -> Result<i64> {
        conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .context("failed to query current migration version")
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
        assert_eq!(MigrationRunner::current_version(&conn).unwrap(), 1);
    }

    #[test]
    fn idempotent_run() {
        let conn = in_memory();
        MigrationRunner::run_pending(&conn).expect("first run");
        MigrationRunner::run_pending(&conn).expect("second run");
        assert_eq!(MigrationRunner::current_version(&conn).unwrap(), 1);
    }

    #[test]
    fn current_version_zero_before_migrations() {
        let conn = in_memory();
        conn.execute_batch(
            "CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            );",
        )
        .unwrap();
        assert_eq!(MigrationRunner::current_version(&conn).unwrap(), 0);
    }

    #[test]
    fn rollback_removes_migration() {
        let conn = in_memory();
        MigrationRunner::run_pending(&conn).expect("run_pending");
        assert_eq!(MigrationRunner::current_version(&conn).unwrap(), 1);

        MigrationRunner::rollback_to(&conn, 0).expect("rollback_to 0");
        assert_eq!(MigrationRunner::current_version(&conn).unwrap(), 0);

        // Tables should be gone.
        assert!(conn
            .prepare("SELECT id FROM players LIMIT 0")
            .is_err());
    }

    #[test]
    fn rollback_skips_unapplied_migration() {
        // rollback_to should silently skip any migration that was never applied.
        // This covers the `if !applied { continue; }` guard in rollback_to.
        let conn = in_memory();
        MigrationRunner::run_pending(&conn).expect("run_pending");

        // Roll back from v1 to v0, then back to v0 again — second call is a no-op.
        MigrationRunner::rollback_to(&conn, 0).expect("rollback_to 0 first time");
        assert_eq!(MigrationRunner::current_version(&conn).unwrap(), 0);

        // v1 is no longer applied; rollback_to should silently skip it.
        MigrationRunner::rollback_to(&conn, 0).expect("rollback_to 0 second time (no-op)");
        assert_eq!(MigrationRunner::current_version(&conn).unwrap(), 0);
    }
}
