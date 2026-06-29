//! Layout metadata read accessors. The canonical `LayoutMeta` lives here and is
//! reused by every consumer (Browse rendering, mode routing, the design canvas)
//! — defined exactly once (see Build Plan: engine accessor ledger).

use anyhow::Result;
use rusqlite::{params, Transaction};

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

/// A layout part (band): header|body|footer|subsummary|grandsummary. Parts stack
/// in `position` order; an object's geometry is relative to its part (#25).
#[derive(Debug, Clone)]
pub struct PartMeta {
    pub id: i64,
    pub layout_id: i64,
    pub kind: String,
    pub height: i64,
    pub position: i64,
}

/// An object on a part: absolute, **part-relative** geometry (the frozen
/// geometry contract) plus a dot-path `binding` like `Customers.Name`. The same
/// objects are rendered live by Browse (#25/#26) and edited by the canvas (#15).
#[derive(Debug, Clone)]
pub struct ObjectMeta {
    pub id: i64,
    pub part_id: i64,
    pub kind: String,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub binding: Option<String>,
    pub props: Option<String>,
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

    /// Parts of a layout, stacked in `position` order (#25).
    pub fn parts(&self, layout_id: i64) -> Result<Vec<PartMeta>> {
        let mut stmt = self.app.prepare(
            "SELECT id, layout_id, kind, height, position FROM meta_part \
             WHERE layout_id=?1 ORDER BY position, id",
        )?;
        let rows = stmt.query_map(params![layout_id], |r| {
            Ok(PartMeta {
                id: r.get(0)?,
                layout_id: r.get(1)?,
                kind: r.get(2)?,
                height: r.get(3)?,
                position: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Objects on a part, in stacking order (#25).
    pub fn objects(&self, part_id: i64) -> Result<Vec<ObjectMeta>> {
        let mut stmt = self.app.prepare(
            "SELECT id, part_id, kind, x, y, w, h, binding, props FROM meta_object \
             WHERE part_id=?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![part_id], |r| {
            Ok(ObjectMeta {
                id: r.get(0)?,
                part_id: r.get(1)?,
                kind: r.get(2)?,
                x: r.get(3)?,
                y: r.get(4)?,
                w: r.get(5)?,
                h: r.get(6)?,
                binding: r.get(7)?,
                props: r.get(8)?,
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

/// Create a default Form layout for a freshly-defined table, inside the caller's
/// transaction (so table + layout are atomic). One meta_layout (view='form'),
/// one body meta_part, and one kind='field' meta_object per field — stacked,
/// with binding `<TableName>.<FieldName>` (the frozen binding contract).
/// Returns the new layout id. (#21)
pub(crate) fn generate_default_form(
    tx: &Transaction<'_>,
    table_id: i64,
    table_name: &str,
    fields: &[(i64, String)],
) -> Result<i64> {
    tx.execute(
        "INSERT INTO meta_layout(name, table_id, view) VALUES (?1, ?2, 'form')",
        params![table_name, table_id],
    )?;
    let layout_id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO meta_part(layout_id, kind, height, position) VALUES (?1, 'body', ?2, 0)",
        params![layout_id, 40 + fields.len() as i64 * 32],
    )?;
    let part_id = tx.last_insert_rowid();
    for (i, (_fid, fname)) in fields.iter().enumerate() {
        let y = 16 + i as i64 * 32;
        let binding = format!("{table_name}.{fname}");
        tx.execute(
            "INSERT INTO meta_object(part_id, kind, x, y, w, h, binding) \
             VALUES (?1, 'field', 16, ?2, 200, 24, ?3)",
            params![part_id, y, binding],
        )?;
    }
    Ok(layout_id)
}

#[cfg(test)]
mod tests {
    use crate::{FieldKind, NewField, Solution};

    #[test]
    fn parts_and_objects_read_the_default_form() {
        // The default Form layout from create_table (#21) is the fixture: one
        // body part, one field object per field, bound `<Table>.<Field>`.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table(
            "Customers",
            &[
                NewField { name: "Name".into(), kind: FieldKind::Text },
                NewField { name: "Email".into(), kind: FieldKind::Text },
            ],
        )
        .unwrap();
        let lay = &s.layouts().unwrap()[0];

        let parts = s.parts(lay.id).unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].kind, "body");
        assert!(parts[0].height > 0);

        let objs = s.objects(parts[0].id).unwrap();
        assert_eq!(objs.len(), 2);
        for o in &objs {
            assert_eq!(o.kind, "field");
            assert_eq!(o.part_id, parts[0].id);
            assert!(o.w > 0 && o.h > 0 && o.x >= 0 && o.y >= 0);
        }
        assert_eq!(objs[0].binding.as_deref(), Some("Customers.Name"));
        assert_eq!(objs[1].binding.as_deref(), Some("Customers.Email"));
        assert!(objs[0].y < objs[1].y, "stacked down the body");

        // unknown ids yield empty, not error
        assert!(s.parts(999_999).unwrap().is_empty());
        assert!(s.objects(999_999).unwrap().is_empty());
    }

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
