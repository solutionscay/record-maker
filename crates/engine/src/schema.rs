//! #7 — the versioned metadata schema + migration runner (ADR-0004).

use anyhow::Result;
use rusqlite::Connection;

/// Ordered metadata migrations as `(name, sql)`. The index **+ 1** is the
/// schema version each migration brings `app.db` to.
///
/// APPEND-ONLY: never edit or reorder a migration that has shipped. The whole
/// point (ADR-0004) is that the contract evolves by adding migrations, never by
/// silently changing an existing one.
const MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_init_meta",
        include_str!("migrations/0001_init_meta.sql"),
    ),
    (
        "0002_layout_contract",
        include_str!("migrations/0002_layout_contract.sql"),
    ),
    (
        "0003_object_content",
        include_str!("migrations/0003_object_content.sql"),
    ),
    (
        "0004_default_header_footer_parts",
        include_str!("migrations/0004_default_header_footer_parts.sql"),
    ),
    (
        "0005_deduplicate_singleton_parts",
        include_str!("migrations/0005_deduplicate_singleton_parts.sql"),
    ),
    (
        "0006_part_props",
        include_str!("migrations/0006_part_props.sql"),
    ),
    (
        "0007_object_groups",
        include_str!("migrations/0007_object_groups.sql"),
    ),
    (
        "0008_schema_notes",
        include_str!("migrations/0008_schema_notes.sql"),
    ),
];

/// Schema version this build targets (the number of migrations defined).
pub fn target_version() -> u32 {
    MIGRATIONS.len() as u32
}

/// Apply any pending metadata migrations to `conn` (an `app.db` connection),
/// each in its own transaction. Idempotent; returns the resulting version.
///
/// The version stamp is SQLite's built-in `PRAGMA user_version`, so the
/// migration mechanism exists from commit #1 — before any solution does.
pub fn migrate(conn: &mut Connection) -> Result<u32> {
    let mut version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    while (version as usize) < MIGRATIONS.len() {
        let (name, sql) = MIGRATIONS[version as usize];
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        // user_version is transactional — rolls back with the migration on error.
        tx.pragma_update(None, "user_version", version + 1)?;
        tx.commit()?;
        version += 1;
        eprintln!("[engine] app.db migrated → v{version} ({name})");
    }
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_creates_meta_tables_and_is_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();

        let v = migrate(&mut conn).unwrap();
        assert_eq!(v, target_version());
        assert!(v >= 1);

        for t in [
            "meta_table",
            "meta_field",
            "meta_relationship",
            "meta_layout",
            "meta_part",
            "meta_object",
        ] {
            let n: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [t],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "missing meta table: {t}");
        }

        // re-running at the current version changes nothing
        assert_eq!(migrate(&mut conn).unwrap(), v);
    }
}
