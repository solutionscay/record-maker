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
        }
        tx.commit()?;

        // Physical table lives in data.db (a separate connection → a separate step).
        let ddl = format!("CREATE TABLE {table_phys} ({})", col_defs.join(", "));
        self.data.execute(&ddl, []).context("create physical table")?;
        Ok(table_id)
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
