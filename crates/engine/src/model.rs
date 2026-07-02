//! #9 — defining user tables/fields and creating their physical tables in data.db.

use anyhow::{Context, Result};
use rusqlite::params;

use crate::Solution;

/// Logical field type. Maps to a SQLite column affinity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Number,
    Date,
    Bool,
}

impl FieldKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FieldKind::Text => "text",
            FieldKind::Number => "number",
            FieldKind::Date => "date",
            FieldKind::Bool => "bool",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "text" => FieldKind::Text,
            "number" => FieldKind::Number,
            "date" => FieldKind::Date,
            "bool" => FieldKind::Bool,
            _ => return None,
        })
    }

    /// SQLite column type (affinity) this kind stores as.
    pub fn sql_type(self) -> &'static str {
        match self {
            FieldKind::Text | FieldKind::Date => "TEXT",
            FieldKind::Number => "REAL",
            FieldKind::Bool => "INTEGER",
        }
    }
}

/// A field to add when defining a table.
#[derive(Debug, Clone)]
pub struct NewField {
    pub name: String,
    pub kind: FieldKind,
}

/// Metadata for a defined user table.
#[derive(Debug, Clone)]
pub struct TableMeta {
    pub id: i64,
    pub name: String,
    /// Physical table name in data.db (always `t_<id>` — a safe identifier).
    pub phys: String,
}

/// Metadata for a field on a user table.
#[derive(Debug, Clone)]
pub struct FieldMeta {
    pub id: i64,
    pub name: String,
    /// Physical column name in data.db (always `f_<id>` — a safe identifier).
    pub phys: String,
    pub kind: FieldKind,
    pub position: i64,
}

impl Solution {
    /// Define a new user table + fields (metadata in app.db) and create its
    /// physical table in data.db. Returns the new table id.
    ///
    /// Physical names are derived from row ids (`t_<id>` / `f_<id>`), so they
    /// are always valid, unique SQL identifiers and need no sanitization — the
    /// only place we interpolate identifiers into SQL strings.
    pub fn create_table(&mut self, name: &str, fields: &[NewField]) -> Result<i64> {
        let tx = self.app.transaction()?;
        tx.execute(
            "INSERT INTO meta_table(name, phys_name) VALUES (?1, '')",
            params![name],
        )
        .context("insert meta_table")?;
        let table_id = tx.last_insert_rowid();
        let table_phys = format!("t_{table_id}");
        tx.execute(
            "UPDATE meta_table SET phys_name=?1 WHERE id=?2",
            params![table_phys, table_id],
        )?;

        let mut col_defs = vec!["id INTEGER PRIMARY KEY".to_string()];
        let mut field_meta: Vec<(i64, String)> = Vec::new();
        for (pos, f) in fields.iter().enumerate() {
            tx.execute(
                "INSERT INTO meta_field(table_id, name, phys_name, kind, position) \
                 VALUES (?1, ?2, '', ?3, ?4)",
                params![table_id, f.name, f.kind.as_str(), pos as i64],
            )?;
            let fid = tx.last_insert_rowid();
            let fphys = format!("f_{fid}");
            tx.execute(
                "UPDATE meta_field SET phys_name=?1 WHERE id=?2",
                params![fphys, fid],
            )?;
            col_defs.push(format!("{fphys} {}", f.kind.sql_type()));
            field_meta.push((fid, f.name.clone()));
        }

        // Default per-view layouts (#21, #57): a Form layout (one body part + a
        // field object per field), cloned into independent List and Table layouts
        // — all in the same transaction, so table + layouts are created atomically.
        // The three start identical but are then designed independently.
        let form_layout_id = crate::layout::generate_default_form(&tx, table_id, name, &field_meta)?;
        crate::layout::clone_layout(&tx, form_layout_id, name, table_id, "list")?;
        crate::layout::clone_layout(&tx, form_layout_id, name, table_id, "table")?;
        tx.commit()?;

        // Physical table lives in data.db (a separate connection → a separate step).
        let ddl = format!("CREATE TABLE {table_phys} ({})", col_defs.join(", "));
        self.data.execute(&ddl, []).context("create physical table")?;
        Ok(table_id)
    }

    /// Add one field to an existing user table, updating metadata and the dynamic
    /// data table. The field is appended at the end of the display order.
    pub fn add_field(&mut self, table_id: i64, f: &NewField) -> Result<FieldMeta> {
        let table = self
            .table_by_id(table_id)?
            .with_context(|| format!("no table {table_id}"))?;
        let position: i64 = self.app.query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM meta_field WHERE table_id=?1",
            [table_id],
            |r| r.get(0),
        )?;
        let tx = self.app.transaction()?;
        tx.execute(
            "INSERT INTO meta_field(table_id, name, phys_name, kind, position) \
             VALUES (?1, ?2, '', ?3, ?4)",
            params![table_id, &f.name, f.kind.as_str(), position],
        )?;
        let fid = tx.last_insert_rowid();
        let fphys = format!("f_{fid}");
        tx.execute(
            "UPDATE meta_field SET phys_name=?1 WHERE id=?2",
            params![fphys, fid],
        )?;
        tx.commit()?;

        let ddl = format!("ALTER TABLE {} ADD COLUMN {fphys} {}", table.phys, f.kind.sql_type());
        self.data.execute(&ddl, []).context("add physical column")?;
        Ok(FieldMeta {
            id: fid,
            name: f.name.clone(),
            phys: fphys,
            kind: f.kind,
            position,
        })
    }

    /// All defined tables, by name.
    pub fn tables(&self) -> Result<Vec<TableMeta>> {
        let mut stmt = self
            .app
            .prepare("SELECT id, name, phys_name FROM meta_table ORDER BY name")?;
        let rows = stmt.query_map([], |r| {
            Ok(TableMeta {
                id: r.get(0)?,
                name: r.get(1)?,
                phys: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Look up a table by its logical name.
    pub fn table_by_name(&self, name: &str) -> Result<Option<TableMeta>> {
        let mut stmt = self
            .app
            .prepare("SELECT id, name, phys_name FROM meta_table WHERE name=?1")?;
        let mut rows = stmt.query_map(params![name], |r| {
            Ok(TableMeta {
                id: r.get(0)?,
                name: r.get(1)?,
                phys: r.get(2)?,
            })
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Look up a table by id.
    pub fn table_by_id(&self, id: i64) -> Result<Option<TableMeta>> {
        let mut stmt = self
            .app
            .prepare("SELECT id, name, phys_name FROM meta_table WHERE id=?1")?;
        let mut rows = stmt.query_map(params![id], |r| {
            Ok(TableMeta {
                id: r.get(0)?,
                name: r.get(1)?,
                phys: r.get(2)?,
            })
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Fields of a table, in display order.
    pub fn fields(&self, table_id: i64) -> Result<Vec<FieldMeta>> {
        let mut stmt = self.app.prepare(
            "SELECT id, name, phys_name, kind, position FROM meta_field \
             WHERE table_id=?1 ORDER BY position, id",
        )?;
        let rows = stmt.query_map(params![table_id], |r| {
            let kind_s: String = r.get(3)?;
            Ok(FieldMeta {
                id: r.get(0)?,
                name: r.get(1)?,
                phys: r.get(2)?,
                kind: FieldKind::parse(&kind_s).unwrap_or(FieldKind::Text),
                position: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests {
    use crate::{FieldKind, NewField, Solution};

    #[test]
    fn create_table_generates_default_form_layout() {
        let mut s = Solution::open_in_memory().unwrap();
        let tid = s
            .create_table(
                "Invoices",
                &[
                    NewField { name: "Number".into(), kind: FieldKind::Text },
                    NewField { name: "Total".into(), kind: FieldKind::Number },
                ],
            )
            .unwrap();

        // Three independent per-view layouts (form/list/table), all bound to the
        // table and sharing its name (#57).
        let layouts = s.layouts_for_table(tid).unwrap();
        assert_eq!(layouts.len(), 3);
        let mut views: Vec<&str> = layouts.iter().map(|l| l.view.as_str()).collect();
        views.sort_unstable();
        assert_eq!(views, ["form", "list", "table"]);
        assert!(layouts.iter().all(|l| l.name == "Invoices" && l.table_id == tid));
        let lay = layouts.iter().find(|l| l.view == "form").unwrap();

        let body_parts: i64 = s
            .app
            .query_row(
                "SELECT count(*) FROM meta_part WHERE layout_id=?1 AND kind='body'",
                [lay.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(body_parts, 1);

        let mut stmt = s
            .app
            .prepare(
                "SELECT o.kind, o.y, o.w, o.h, o.binding, o.content FROM meta_object o \
                 JOIN meta_part p ON p.id = o.part_id WHERE p.layout_id = ?1 \
                 ORDER BY o.y, o.x",
            )
            .unwrap();
        let rows: Vec<(String, i64, i64, i64, Option<String>, Option<String>)> = stmt
            .query_map([lay.id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();

        // Per field: a label text object (caption in `content`) then a value field
        // object (its `binding`), the label to the left on the same row (#60).
        assert_eq!(rows.len(), 4);
        for row in &rows {
            assert!(row.2 > 0 && row.3 > 0, "non-zero w/h");
        }
        assert_eq!((rows[0].0.as_str(), rows[0].5.as_deref()), ("text", Some("Number")));
        assert_eq!((rows[1].0.as_str(), rows[1].4.as_deref()), ("field", Some("Invoices.Number")));
        assert_eq!((rows[2].0.as_str(), rows[2].5.as_deref()), ("text", Some("Total")));
        assert_eq!((rows[3].0.as_str(), rows[3].4.as_deref()), ("field", Some("Invoices.Total")));
        assert!(rows[0].1 == rows[1].1 && rows[2].1 == rows[3].1, "label shares its value's row");
        assert!(rows[1].1 < rows[3].1, "rows increase down the form");
    }

    #[test]
    fn add_field_appends_metadata_and_physical_column() {
        let mut s = Solution::open_in_memory().unwrap();
        let tid = s
            .create_table(
                "Customers",
                &[NewField {
                    name: "Name".into(),
                    kind: FieldKind::Text,
                }],
            )
            .unwrap();

        let added = s
            .add_field(
                tid,
                &NewField {
                    name: "Age".into(),
                    kind: FieldKind::Number,
                },
            )
            .unwrap();
        assert_eq!(added.name, "Age");
        assert_eq!(added.position, 1);

        let table = s.table_by_name("Customers").unwrap().unwrap();
        let fields = s.fields(tid).unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[1].name, "Age");

        s.insert_record(
            &table,
            &[(&fields[0], "Ada".into()), (&fields[1], "36".into())],
        )
        .unwrap();
        let rec = s.list_records(&table, &fields).unwrap();
        assert_eq!(rec[0].cells, vec!["Ada".to_string(), "36".to_string()]);
    }

    #[test]
    fn zero_field_table_gets_layout_and_body_but_no_objects() {
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table("Empty", &[]).unwrap();
        let lay = &s.layouts().unwrap()[0];
        let objs: i64 = s
            .app
            .query_row(
                "SELECT count(*) FROM meta_object o JOIN meta_part p ON p.id = o.part_id \
                 WHERE p.layout_id = ?1",
                [lay.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(objs, 0);
    }
}
