//! #8 — the two-database connection layer (ADR-0002).

use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

use crate::schema;

/// An open record-maker solution: the two SQLite databases from ADR-0002.
///
/// * [`Solution::app`] holds the metadata (fixed, versioned schema).
/// * [`Solution::data`] holds the user's tables (dynamic schema).
///
/// Keeping them in separate files lets the app definition be shipped/published
/// without the data, and keeps their very different schema lifecycles apart.
pub struct Solution {
    /// Metadata database (`app.db`).
    pub app: Connection,
    /// User-data database (`data.db`).
    pub data: Connection,
}

impl Solution {
    /// Open (creating if absent) a solution stored under `dir`, then bring the
    /// metadata schema up to date.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let app = open_file(dir.join("app.db"))?;
        let data = open_file(dir.join("data.db"))?;
        Self::finish(app, data)
    }

    /// Open an in-memory solution (used by tests).
    pub fn open_in_memory() -> Result<Self> {
        let app = configure(Connection::open_in_memory()?)?;
        let data = configure(Connection::open_in_memory()?)?;
        Self::finish(app, data)
    }

    fn finish(mut app: Connection, data: Connection) -> Result<Self> {
        schema::migrate(&mut app)?;
        let mut sol = Self { app, data };
        // Backfill the per-view layout split (#57) for any table predating it.
        sol.ensure_view_layouts()?;
        Ok(sol)
    }

    /// Current metadata schema version of this solution's `app.db`.
    pub fn schema_version(&self) -> Result<u32> {
        Ok(self.app.query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }
}

fn open_file(path: impl AsRef<Path>) -> Result<Connection> {
    configure(Connection::open(path)?)
}

fn configure(conn: Connection) -> Result<Connection> {
    // WAL is ignored for in-memory dbs; foreign keys must be enabled per-connection.
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_in_memory_runs_migrations_and_separates_dbs() {
        let s = Solution::open_in_memory().unwrap();

        // app.db is migrated to the current target version
        assert_eq!(s.schema_version().unwrap(), schema::target_version());

        // foreign keys enforced on both connections
        let fk_app: i64 = s.app.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap();
        let fk_data: i64 = s.data.query_row("PRAGMA foreign_keys", [], |r| r.get(0)).unwrap();
        assert_eq!((fk_app, fk_data), (1, 1));

        // data.db starts with no user tables — the meta schema lives only in app.db
        let user_tables: i64 = s
            .data
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(user_tables, 0);
    }
}
