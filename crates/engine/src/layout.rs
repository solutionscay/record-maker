//! Layout metadata read accessors. The canonical `LayoutMeta` lives here and is
//! reused by every consumer (Browse rendering, mode routing, the design canvas)
//! — defined exactly once (see Build Plan: engine accessor ledger).

use anyhow::Result;
use rusqlite::params;

use crate::Solution;

/// Metadata for a layout. A layout binds to a primary table (ADR-0003) and is
/// rendered live by Browse and edited by Layout mode (ADR-0005).
#[derive(Debug, Clone)]
pub struct LayoutMeta {
    pub id: i64,
    pub name: String,
    pub table_id: i64,
    pub view: String,
}

impl Solution {
    /// All layouts, ordered by name.
    pub fn layouts(&self) -> Result<Vec<LayoutMeta>> {
        let mut stmt = self
            .app
            .prepare("SELECT id, name, table_id, view FROM meta_layout ORDER BY name, id")?;
        let rows = stmt.query_map([], |r| {
            Ok(LayoutMeta {
                id: r.get(0)?,
                name: r.get(1)?,
                table_id: r.get(2)?,
                view: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Look up a single layout by id.
    pub fn layout_by_id(&self, id: i64) -> Result<Option<LayoutMeta>> {
        let mut stmt = self
            .app
            .prepare("SELECT id, name, table_id, view FROM meta_layout WHERE id=?1")?;
        let mut rows = stmt.query_map(params![id], |r| {
            Ok(LayoutMeta {
                id: r.get(0)?,
                name: r.get(1)?,
                table_id: r.get(2)?,
                view: r.get(3)?,
            })
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Solution;

    #[test]
    fn layouts_empty_then_returns_inserted() {
        let s = Solution::open_in_memory().unwrap();
        assert!(s.layouts().unwrap().is_empty());

        s.app
            .execute("INSERT INTO meta_table(name, phys_name) VALUES ('T','t_x')", [])
            .unwrap();
        let tid = s.app.last_insert_rowid();
        s.app
            .execute(
                "INSERT INTO meta_layout(name, table_id, view) VALUES ('T', ?1, 'form')",
                [tid],
            )
            .unwrap();

        let ls = s.layouts().unwrap();
        assert_eq!(ls.len(), 1);
        assert_eq!(ls[0].name, "T");
        assert_eq!(ls[0].table_id, tid);
        assert_eq!(ls[0].view, "form");

        let one = s.layout_by_id(ls[0].id).unwrap().unwrap();
        assert_eq!(one.id, ls[0].id);
        assert!(s.layout_by_id(999_999).unwrap().is_none());
    }
}
