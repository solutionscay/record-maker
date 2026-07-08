//! #9 — defining user tables/fields and creating their physical tables in data.db.

use anyhow::{bail, Context, Result};
use rusqlite::params;
use rusqlite::types::ValueRef;
use serde_json::Value;

use crate::options::{FieldOptions, FieldReference, FieldReferenceError};
use crate::Solution;

/// Logical field type. Maps to a SQLite column affinity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Number,
    /// Date only, stored as ISO-8601 TEXT (`YYYY-MM-DD`).
    Date,
    Bool,
    /// Time-of-day only, stored as ISO-8601 TEXT (`HH:MM:SS`).
    Time,
    /// Date + time, stored as ISO-8601 TEXT (`YYYY-MM-DDTHH:MM:SS`).
    Timestamp,
}

impl FieldKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FieldKind::Text => "text",
            FieldKind::Number => "number",
            FieldKind::Date => "date",
            FieldKind::Bool => "bool",
            FieldKind::Time => "time",
            FieldKind::Timestamp => "timestamp",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "text" => FieldKind::Text,
            "number" => FieldKind::Number,
            "date" => FieldKind::Date,
            "bool" => FieldKind::Bool,
            "time" => FieldKind::Time,
            "timestamp" => FieldKind::Timestamp,
            _ => return None,
        })
    }

    /// SQLite column type (affinity) this kind stores as.
    pub fn sql_type(self) -> &'static str {
        match self {
            FieldKind::Text | FieldKind::Date | FieldKind::Time | FieldKind::Timestamp => "TEXT",
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
    pub notes: String,
    /// Physical table name in data.db (always `t_<id>` — a safe identifier).
    pub phys: String,
    pub position: i64,
}

/// Metadata for a field on a user table.
#[derive(Debug, Clone)]
pub struct FieldMeta {
    pub id: i64,
    pub name: String,
    pub notes: String,
    /// Physical column name in data.db (always `f_<id>` — a safe identifier).
    pub phys: String,
    pub kind: FieldKind,
    /// Raw JSON options bag from `meta_field.options`.
    pub options: String,
    pub position: i64,
}

/// Metadata for a named foreign-key relationship between two user tables.
#[derive(Debug, Clone)]
pub struct RelationshipMeta {
    pub id: i64,
    pub name: String,
    /// Child/source table: the table that owns the FK field.
    pub from_table: i64,
    /// Parent/target table: the table whose key field is referenced.
    pub to_table: i64,
    /// FK field on [`RelationshipMeta::from_table`].
    pub from_field: i64,
    /// Key field on [`RelationshipMeta::to_table`].
    pub to_field: i64,
}

/// A relationship to create or replace.
#[derive(Debug, Clone)]
pub struct NewRelationship {
    pub name: String,
    pub from_table: i64,
    pub to_table: i64,
    pub from_field: i64,
    pub to_field: i64,
}

/// Metadata for a reusable value list.
#[derive(Debug, Clone)]
pub struct ValueListMeta {
    pub id: i64,
    pub name: String,
    pub source: String,
    pub config: String,
    pub position: i64,
}

/// A value list to create or replace.
#[derive(Debug, Clone)]
pub struct NewValueList {
    pub name: String,
    pub source: String,
    pub config: String,
}

/// One resolved value-list item. Dividers are skipped by validation consumers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueListItem {
    pub value: String,
    pub display: String,
    pub divider: bool,
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
        let next_pos: i64 = tx
            .query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM meta_table", [], |r| r.get(0))
            .context("get next table position")?;
        tx.execute(
            "INSERT INTO meta_table(name, phys_name, position) VALUES (?1, '', ?2)",
            params![name, next_pos],
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

        // System primary key (#156): every table carries one auto-minted, immutable
        // UUID field. Created LAST so the user fields above keep their ids/positions,
        // and deliberately NOT pushed into `field_meta`, so the default layouts don't
        // place this read-only key. `system` in its options marks it undeletable /
        // fixed-kind / value-immutable; the physical column is UNIQUE as a backstop.
        let used: std::collections::HashSet<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        let pk_name = ["ID", "Record ID", "System ID", "UID"]
            .into_iter()
            .find(|n| !used.contains(n))
            .unwrap_or("PK");
        // Position -1 keeps the PK sorting first in `all_fields` and, crucially,
        // out of the user field 0..n-1 sequence — so `add_field`'s MAX(position)+1
        // and the user field indices/positions are exactly as they were pre-#156.
        tx.execute(
            "INSERT INTO meta_field(table_id, name, phys_name, kind, position, options) \
             VALUES (?1, ?2, '', ?3, -1, '{\"system\":true}')",
            params![table_id, pk_name, FieldKind::Text.as_str()],
        )?;
        let pk_id = tx.last_insert_rowid();
        let pk_phys = format!("f_{pk_id}");
        tx.execute(
            "UPDATE meta_field SET phys_name=?1 WHERE id=?2",
            params![pk_phys, pk_id],
        )?;
        col_defs.push(format!("{pk_phys} TEXT UNIQUE"));

        // Default per-view layouts (#21, #57): a Form layout (one body part + a
        // field object per field), cloned into independent List and Table layouts
        // — all in the same transaction, so table + layouts are created atomically.
        // The three start identical but are then designed independently.
        let form_layout_id =
            crate::layout::generate_default_form(&tx, table_id, name, name, "form", &field_meta)?;
        crate::layout::clone_layout(&tx, form_layout_id, name, table_id, "list")?;
        crate::layout::clone_layout(&tx, form_layout_id, name, table_id, "table")?;
        // Mark the trio as the table's default layouts (#151): enable/disable-able
        // but never deletable, unlike Layout Manager "New layout" ones. At this
        // point the trio is the only thing bound to the table, so this is safe.
        tx.execute(
            "UPDATE meta_layout SET is_default=1 WHERE table_id=?1",
            params![table_id],
        )?;
        tx.commit()?;

        // Physical table lives in data.db (a separate connection → a separate step).
        let ddl = format!("CREATE TABLE {table_phys} ({})", col_defs.join(", "));
        self.data
            .execute(&ddl, [])
            .context("create physical table")?;
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

        let ddl = format!(
            "ALTER TABLE {} ADD COLUMN {fphys} {}",
            table.phys,
            f.kind.sql_type()
        );
        self.data.execute(&ddl, []).context("add physical column")?;
        Ok(FieldMeta {
            id: fid,
            name: f.name.clone(),
            notes: String::new(),
            phys: fphys,
            kind: f.kind,
            options: String::new(),
            position,
        })
    }

    /// Rename a user table in metadata. Physical table names stay id-derived and
    /// stable; direct layout bindings rooted at the old logical table name are
    /// rewritten so existing field objects continue to resolve.
    pub fn rename_table(&mut self, table_id: i64, name: &str) -> Result<Option<TableMeta>> {
        let Some(table) = self.table_by_id(table_id)? else {
            return Ok(None);
        };
        self.update_table(table_id, name, &table.notes)
    }

    /// Update a user table's editable metadata. Renaming rewrites direct layout
    /// bindings rooted at the old logical table name.
    pub fn update_table(
        &mut self,
        table_id: i64,
        name: &str,
        notes: &str,
    ) -> Result<Option<TableMeta>> {
        let Some(table) = self.table_by_id(table_id)? else {
            return Ok(None);
        };
        let old_name = table.name;
        let tx = self.app.transaction()?;
        tx.execute(
            "UPDATE meta_table SET name=?1, notes=?2 WHERE id=?3",
            params![name, notes, table_id],
        )?;
        if old_name != name {
            tx.execute(
                "UPDATE meta_layout SET name=?1 WHERE table_id=?2",
                params![name, table_id],
            )?;
            let prefix_len = old_name.len() as i64 + 1;
            let old_prefix = format!("{old_name}.");
            tx.execute(
                "UPDATE meta_object SET binding=?1 || substr(binding, ?2) \
                 WHERE binding=?3 OR substr(binding, 1, ?2)=?4",
                params![name, prefix_len, old_name, old_prefix],
            )?;
        }
        tx.commit()?;
        self.table_by_id(table_id)
    }

    /// Delete a user table and its metadata. The data table is dropped first, then
    /// metadata cascades remove fields, layouts, and relationships.
    pub fn delete_table(&mut self, table_id: i64) -> Result<usize> {
        let Some(table) = self.table_by_id(table_id)? else {
            return Ok(0);
        };
        self.data
            .execute(&format!("DROP TABLE IF EXISTS {}", table.phys), [])
            .context("drop physical table")?;
        let n = self
            .app
            .execute("DELETE FROM meta_table WHERE id=?1", params![table_id])?;
        Ok(n)
    }

    /// Rename a field and rewrite direct `<Table>.<Field>` bindings.
    pub fn rename_field(
        &mut self,
        table_id: i64,
        field_id: i64,
        name: &str,
    ) -> Result<Option<FieldMeta>> {
        let Some(field) = self.field_by_id(table_id, field_id)? else {
            return Ok(None);
        };
        self.update_field(table_id, field_id, name, field.kind, &field.notes)
    }

    /// Update a field's editable metadata. Retyping rebuilds the physical table;
    /// renaming rewrites direct `<Table>.<Field>` bindings.
    pub fn update_field(
        &mut self,
        table_id: i64,
        field_id: i64,
        name: &str,
        kind: FieldKind,
        notes: &str,
    ) -> Result<Option<FieldMeta>> {
        let Some(table) = self.table_by_id(table_id)? else {
            return Ok(None);
        };
        let Some(field) = self.field_by_id(table_id, field_id)? else {
            return Ok(None);
        };
        // The system primary key (#156) has a fixed kind — only its name is editable.
        if FieldOptions::parse(&field.options).system && field.kind != kind {
            bail!("the system primary key's type cannot be changed");
        }
        if field.kind != kind {
            // The rebuild must recreate EVERY physical column, including the
            // system PK, so use all_fields (not the user-only fields).
            let mut fields = self.all_fields(table_id)?;
            for f in &mut fields {
                if f.id == field_id {
                    f.kind = kind;
                }
            }
            self.rebuild_physical_table(&table, &fields)
                .context("rebuild physical table for field retype")?;
        }
        let tx = self.app.transaction()?;
        tx.execute(
            "UPDATE meta_field SET name=?1, kind=?2, notes=?3 WHERE id=?4 AND table_id=?5",
            params![name, kind.as_str(), notes, field_id, table_id],
        )?;
        if field.name != name {
            let old_binding = format!("{}.{}", table.name, field.name);
            let new_binding = format!("{}.{}", table.name, name);
            tx.execute(
                "UPDATE meta_object SET binding=?1 WHERE binding=?2",
                params![new_binding, old_binding],
            )?;
        }
        tx.commit()?;
        self.field_by_id(table_id, field_id)
    }

    /// Update a field's raw JSON options bag. The engine stores this opaquely;
    /// server/UI code owns the shape for validation, auto-enter, and future rules.
    pub fn update_field_options(
        &self,
        table_id: i64,
        field_id: i64,
        options: &str,
    ) -> Result<Option<FieldMeta>> {
        let n = self.app.execute(
            "UPDATE meta_field SET options=?1 WHERE id=?2 AND table_id=?3",
            params![options, field_id, table_id],
        )?;
        if n == 0 {
            return Ok(None);
        }
        self.field_by_id(table_id, field_id)
    }

    /// Retype a field. SQLite cannot alter a column type in place, so the dynamic
    /// table is rebuilt with the same physical columns and the new affinity.
    pub fn retype_field(
        &mut self,
        table_id: i64,
        field_id: i64,
        kind: FieldKind,
    ) -> Result<Option<FieldMeta>> {
        let Some(table) = self.table_by_id(table_id)? else {
            return Ok(None);
        };
        let Some(field) = self.field_by_id(table_id, field_id)? else {
            return Ok(None);
        };
        // The system primary key (#156) cannot be retyped.
        if FieldOptions::parse(&field.options).system {
            bail!("the system primary key's type cannot be changed");
        }
        if field.kind != kind {
            // The rebuild must recreate EVERY physical column, including the
            // system PK, so use all_fields (not the user-only fields).
            let mut fields = self.all_fields(table_id)?;
            for f in &mut fields {
                if f.id == field_id {
                    f.kind = kind;
                }
            }
            self.rebuild_physical_table(&table, &fields)
                .context("rebuild physical table for field retype")?;
        }
        self.app.execute(
            "UPDATE meta_field SET kind=?1 WHERE id=?2 AND table_id=?3",
            params![kind.as_str(), field_id, table_id],
        )?;
        self.field_by_id(table_id, field_id)
    }

    /// Reorder every field in a table. `field_ids` must contain exactly the
    /// table's current fields, once each.
    pub fn reorder_fields(&mut self, table_id: i64, field_ids: &[i64]) -> Result<Vec<FieldMeta>> {
        let current = self.fields(table_id)?;
        if current.len() != field_ids.len() {
            bail!("field order must include every field exactly once");
        }
        for f in &current {
            if !field_ids.contains(&f.id) {
                bail!("field order must include every field exactly once");
            }
        }
        for id in field_ids {
            if field_ids.iter().filter(|other| *other == id).count() != 1 {
                bail!("field order must not contain duplicates");
            }
        }
        let tx = self.app.transaction()?;
        for (position, field_id) in field_ids.iter().enumerate() {
            tx.execute(
                "UPDATE meta_field SET position=?1 WHERE id=?2 AND table_id=?3",
                params![position as i64, field_id, table_id],
            )?;
        }
        tx.commit()?;
        self.fields(table_id)
    }

    /// Reorder the flat tables list (#162): `table_ids` must include
    /// every table in the solution, exactly once.
    pub fn reorder_tables(&mut self, table_ids: &[i64]) -> Result<Vec<TableMeta>> {
        let current = self.tables()?;
        if current.len() != table_ids.len() {
            bail!("table order must include every table exactly once");
        }
        for t in &current {
            if !table_ids.contains(&t.id) {
                bail!("table order must include every table exactly once");
            }
        }
        for id in table_ids {
            if table_ids.iter().filter(|other| *other == id).count() != 1 {
                bail!("table order must not contain duplicates");
            }
        }
        let tx = self.app.transaction()?;
        for (position, table_id) in table_ids.iter().enumerate() {
            tx.execute(
                "UPDATE meta_table SET position=?1 WHERE id=?2",
                params![position as i64, table_id],
            )?;
        }
        tx.commit()?;
        self.tables()
    }

    /// Delete a field from metadata and from the physical data table.
    pub fn delete_field(&mut self, table_id: i64, field_id: i64) -> Result<usize> {
        let Some(table) = self.table_by_id(table_id)? else {
            return Ok(0);
        };
        let Some(field) = self.field_by_id(table_id, field_id)? else {
            return Ok(0);
        };
        // The system primary key (#156) is undeletable.
        if FieldOptions::parse(&field.options).system {
            bail!("the system primary key cannot be deleted");
        }
        self.data
            .execute(
                &format!("ALTER TABLE {} DROP COLUMN {}", table.phys, field.phys),
                [],
            )
            .context("drop physical column")?;
        let binding = format!("{}.{}", table.name, field.name);
        let tx = self.app.transaction()?;
        tx.execute(
            "DELETE FROM meta_relationship WHERE from_field=?1 OR to_field=?1",
            params![field_id],
        )?;
        tx.execute(
            "UPDATE meta_object SET binding=NULL WHERE binding=?1",
            params![binding],
        )?;
        let n = tx.execute(
            "DELETE FROM meta_field WHERE id=?1 AND table_id=?2",
            params![field_id, table_id],
        )?;
        tx.execute(
            "UPDATE meta_field SET position = ( \
                 SELECT count(*) FROM meta_field f2 \
                 WHERE f2.table_id=meta_field.table_id \
                   AND (f2.position < meta_field.position \
                        OR (f2.position = meta_field.position AND f2.id <= meta_field.id)) \
             ) - 1 WHERE table_id=?1",
            params![table_id],
        )?;
        tx.commit()?;
        Ok(n)
    }

    /// All defined tables, by position and id.
    pub fn tables(&self) -> Result<Vec<TableMeta>> {
        let mut stmt = self
            .app
            .prepare("SELECT id, name, notes, phys_name, position FROM meta_table ORDER BY position, id")?;
        let rows = stmt.query_map([], |r| {
            Ok(TableMeta {
                id: r.get(0)?,
                name: r.get(1)?,
                notes: r.get(2)?,
                phys: r.get(3)?,
                position: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Look up a table by its logical name.
    pub fn table_by_name(&self, name: &str) -> Result<Option<TableMeta>> {
        let mut stmt = self
            .app
            .prepare("SELECT id, name, notes, phys_name, position FROM meta_table WHERE name=?1")?;
        let mut rows = stmt.query_map(params![name], |r| {
            Ok(TableMeta {
                id: r.get(0)?,
                name: r.get(1)?,
                notes: r.get(2)?,
                phys: r.get(3)?,
                position: r.get(4)?,
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
            .prepare("SELECT id, name, notes, phys_name, position FROM meta_table WHERE id=?1")?;
        let mut rows = stmt.query_map(params![id], |r| {
            Ok(TableMeta {
                id: r.get(0)?,
                name: r.get(1)?,
                notes: r.get(2)?,
                phys: r.get(3)?,
                position: r.get(4)?,
            })
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Look up a field by id, scoped to its table.
    pub fn field_by_id(&self, table_id: i64, field_id: i64) -> Result<Option<FieldMeta>> {
        let mut stmt = self.app.prepare(
            "SELECT id, name, notes, phys_name, kind, COALESCE(options, ''), position FROM meta_field \
             WHERE table_id=?1 AND id=?2",
        )?;
        let mut rows = stmt.query_map(params![table_id, field_id], |r| {
            let kind_s: String = r.get(4)?;
            Ok(FieldMeta {
                id: r.get(0)?,
                name: r.get(1)?,
                notes: r.get(2)?,
                phys: r.get(3)?,
                kind: FieldKind::parse(&kind_s).unwrap_or(FieldKind::Text),
                options: r.get(5)?,
                position: r.get(6)?,
            })
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Every field of a table INCLUDING the system primary key (#156), in display
    /// order. Use this for physical operations (insert/select/table rebuild) and
    /// the schema field list; use [`fields`](Self::fields) for the user-managed
    /// fields that layouts and reordering operate on.
    pub fn all_fields(&self, table_id: i64) -> Result<Vec<FieldMeta>> {
        let mut stmt = self.app.prepare(
            "SELECT id, name, notes, phys_name, kind, COALESCE(options, ''), position FROM meta_field \
             WHERE table_id=?1 ORDER BY position, id",
        )?;
        let rows = stmt.query_map(params![table_id], |r| {
            let kind_s: String = r.get(4)?;
            Ok(FieldMeta {
                id: r.get(0)?,
                name: r.get(1)?,
                notes: r.get(2)?,
                phys: r.get(3)?,
                kind: FieldKind::parse(&kind_s).unwrap_or(FieldKind::Text),
                options: r.get(5)?,
                position: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The user-managed fields, in display order — EXCLUDES the system primary key
    /// (#156). This is the field set layouts place, browse columns show, and
    /// reordering operates on, so its shape is unchanged from before the system PK.
    pub fn fields(&self, table_id: i64) -> Result<Vec<FieldMeta>> {
        Ok(self
            .all_fields(table_id)?
            .into_iter()
            .filter(|f| !FieldOptions::parse(&f.options).system)
            .collect())
    }

    /// All value lists, in manager display order.
    pub fn value_lists(&self) -> Result<Vec<ValueListMeta>> {
        let mut stmt = self.app.prepare(
            "SELECT id, name, source, config, position \
             FROM meta_value_list ORDER BY position, id",
        )?;
        let rows = stmt.query_map([], value_list_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Look up one value list by id.
    pub fn value_list_by_id(&self, id: i64) -> Result<Option<ValueListMeta>> {
        let mut stmt = self.app.prepare(
            "SELECT id, name, source, config, position \
             FROM meta_value_list WHERE id=?1",
        )?;
        let mut rows = stmt.query_map(params![id], value_list_from_row)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Create a value list. The engine validates source/config only enough to
    /// keep the metadata contract coherent; consumers resolve it at read time.
    pub fn create_value_list(&mut self, list: &NewValueList) -> Result<ValueListMeta> {
        validate_value_list_source(&list.source)?;
        validate_value_list_config(&list.source, &list.config)?;
        let position: i64 = self.app.query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM meta_value_list",
            [],
            |r| r.get(0),
        )?;
        self.app.execute(
            "INSERT INTO meta_value_list(name, source, config, position) \
             VALUES (?1, ?2, ?3, ?4)",
            params![list.name, list.source, list.config, position],
        )?;
        Ok(self
            .value_list_by_id(self.app.last_insert_rowid())?
            .expect("inserted value list"))
    }

    /// Replace a value-list declaration.
    pub fn update_value_list(
        &mut self,
        id: i64,
        list: &NewValueList,
    ) -> Result<Option<ValueListMeta>> {
        validate_value_list_source(&list.source)?;
        validate_value_list_config(&list.source, &list.config)?;
        let n = self.app.execute(
            "UPDATE meta_value_list SET name=?1, source=?2, config=?3 WHERE id=?4",
            params![list.name, list.source, list.config, id],
        )?;
        if n == 0 {
            return Ok(None);
        }
        self.value_list_by_id(id)
    }

    /// Duplicate a value list, returning the new copy with a unique name.
    pub fn duplicate_value_list(
        &mut self,
        id: i64,
        name: Option<&str>,
    ) -> Result<Option<ValueListMeta>> {
        let Some(src) = self.value_list_by_id(id)? else {
            return Ok(None);
        };
        let name = match name {
            Some(name) if !name.trim().is_empty() => name.trim().to_string(),
            _ => self.unique_value_list_copy_name(&src.name)?,
        };
        self.create_value_list(&NewValueList {
            name,
            source: src.source,
            config: src.config,
        })
        .map(Some)
    }

    /// Delete a value list by id.
    pub fn delete_value_list(&self, id: i64) -> Result<usize> {
        Ok(self
            .app
            .execute("DELETE FROM meta_value_list WHERE id=?1", params![id])?)
    }

    /// Resolve a value list to concrete ordered items.
    pub fn resolve_value_list(&self, id: i64) -> Result<Option<Vec<ValueListItem>>> {
        let Some(list) = self.value_list_by_id(id)? else {
            return Ok(None);
        };
        match list.source.as_str() {
            "custom" => Ok(Some(resolve_custom_value_list(&list.config)?)),
            "field" => Ok(Some(self.resolve_field_value_list(&list.config)?)),
            other => bail!("unsupported value-list source {other}"),
        }
    }

    fn unique_value_list_copy_name(&self, base: &str) -> Result<String> {
        let mut candidate = format!("{base} Copy");
        let mut suffix = 2;
        while value_list_name_exists(self, &candidate)? {
            candidate = format!("{base} Copy {suffix}");
            suffix += 1;
        }
        Ok(candidate)
    }

    fn resolve_field_value_list(&self, config: &str) -> Result<Vec<ValueListItem>> {
        let config: Value =
            serde_json::from_str(config).context("parse field value-list config")?;
        let from_field = config
            .get("fromField")
            .and_then(Value::as_i64)
            .context("field value list needs fromField")?;
        let second_field = config.get("secondField").and_then(Value::as_i64);
        let show_second_only = config
            .get("showSecondOnly")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let sort = config
            .get("sort")
            .and_then(Value::as_str)
            .unwrap_or("first");
        let (table, first) = self
            .field_table_by_field_id(from_field)?
            .with_context(|| format!("unknown value-list field {from_field}"))?;
        let second = match second_field {
            Some(field_id) => Some(self.field_by_id(table.id, field_id)?.with_context(|| {
                format!("second field {field_id} is not on table {}", table.id)
            })?),
            None => None,
        };
        let order = if sort == "second" && second.is_some() {
            "second_value, first_value"
        } else {
            "first_value, second_value"
        };
        let sql = match &second {
            Some(second) => format!(
                "SELECT DISTINCT {first} AS first_value, {second} AS second_value FROM {table} ORDER BY {order}",
                first = first.phys,
                second = second.phys,
                table = table.phys
            ),
            None => format!(
                "SELECT DISTINCT {first} AS first_value, NULL AS second_value FROM {table} ORDER BY {order}",
                first = first.phys,
                table = table.phys
            ),
        };
        let mut stmt = self.data.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let value = value_ref_to_string(row.get_ref(0)?);
            let second = value_ref_to_string(row.get_ref(1)?);
            let display = if show_second_only && !second.is_empty() {
                second.clone()
            } else if second.is_empty() {
                value.clone()
            } else {
                format!("{value} {second}")
            };
            Ok(ValueListItem {
                value,
                display,
                divider: false,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn field_table_by_field_id(&self, field_id: i64) -> Result<Option<(TableMeta, FieldMeta)>> {
        let mut stmt = self.app.prepare(
            "SELECT t.id, t.name, COALESCE(t.notes, ''), t.phys_name, t.position, \
                    f.id, f.name, COALESCE(f.notes, ''), f.phys_name, f.kind, COALESCE(f.options, ''), f.position \
             FROM meta_field f JOIN meta_table t ON t.id=f.table_id WHERE f.id=?1",
        )?;
        let mut rows = stmt.query_map(params![field_id], |r| {
            let kind_s: String = r.get(9)?;
            Ok((
                TableMeta {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    notes: r.get(2)?,
                    phys: r.get(3)?,
                    position: r.get(4)?,
                },
                FieldMeta {
                    id: r.get(5)?,
                    name: r.get(6)?,
                    notes: r.get(7)?,
                    phys: r.get(8)?,
                    kind: FieldKind::parse(&kind_s).unwrap_or(FieldKind::Text),
                    options: r.get(10)?,
                    position: r.get(11)?,
                },
            ))
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// All declared relationships, ordered by source table then name.
    pub fn relationships(&self) -> Result<Vec<RelationshipMeta>> {
        let mut stmt = self.app.prepare(
            "SELECT id, name, from_table, to_table, from_field, to_field \
             FROM meta_relationship ORDER BY from_table, name, id",
        )?;
        let rows = stmt.query_map([], relationship_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Declared relationships whose FK/source side is `from_table`.
    pub fn relationships_from_table(&self, from_table: i64) -> Result<Vec<RelationshipMeta>> {
        let mut stmt = self.app.prepare(
            "SELECT id, name, from_table, to_table, from_field, to_field \
             FROM meta_relationship WHERE from_table=?1 ORDER BY from_table, name, id",
        )?;
        let rows = stmt.query_map(params![from_table], relationship_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Look up one relationship by id.
    pub fn relationship_by_id(&self, id: i64) -> Result<Option<RelationshipMeta>> {
        let mut stmt = self.app.prepare(
            "SELECT id, name, from_table, to_table, from_field, to_field \
             FROM meta_relationship WHERE id=?1",
        )?;
        let mut rows = stmt.query_map(params![id], relationship_from_row)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Declare a named FK relationship. Returns `None` when any referenced table
    /// or field does not exist, or when a field is not on the declared table.
    ///
    /// The source field's options `reference` key is stamped in the SAME
    /// transaction, so the options bag and the relationship row cannot disagree.
    pub fn create_relationship(
        &mut self,
        rel: &NewRelationship,
    ) -> Result<Option<RelationshipMeta>> {
        if !self.relationship_refs_are_valid(rel)? {
            return Ok(None);
        }
        let source = self
            .field_by_id(rel.from_table, rel.from_field)?
            .context("validated source field")?;
        let tx = self.app.transaction()?;
        tx.execute(
            "INSERT INTO meta_relationship(name, from_table, to_table, from_field, to_field) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                rel.name,
                rel.from_table,
                rel.to_table,
                rel.from_field,
                rel.to_field
            ],
        )?;
        let id = tx.last_insert_rowid();
        let saved = RelationshipMeta {
            id,
            name: rel.name.clone(),
            from_table: rel.from_table,
            to_table: rel.to_table,
            from_field: rel.from_field,
            to_field: rel.to_field,
        };
        stamp_reference(&tx, &source, Some(&saved))?;
        tx.commit()?;
        self.relationship_by_id(id)
    }

    /// Replace a relationship declaration.
    ///
    /// Options stay in step transactionally: the new source field's `reference`
    /// key is stamped, and when the FK side moved, the old source field's key is
    /// cleared — all with the row update itself.
    pub fn update_relationship(
        &mut self,
        id: i64,
        rel: &NewRelationship,
    ) -> Result<Option<RelationshipMeta>> {
        let Some(old) = self.relationship_by_id(id)? else {
            return Ok(None);
        };
        if !self.relationship_refs_are_valid(rel)? {
            return Ok(None);
        }
        let source = self
            .field_by_id(rel.from_table, rel.from_field)?
            .context("validated source field")?;
        let moved = (old.from_table, old.from_field) != (rel.from_table, rel.from_field);
        let old_source = if moved {
            self.field_by_id(old.from_table, old.from_field)?
        } else {
            None
        };
        let saved = RelationshipMeta {
            id,
            name: rel.name.clone(),
            from_table: rel.from_table,
            to_table: rel.to_table,
            from_field: rel.from_field,
            to_field: rel.to_field,
        };
        let tx = self.app.transaction()?;
        tx.execute(
            "UPDATE meta_relationship \
             SET name=?1, from_table=?2, to_table=?3, from_field=?4, to_field=?5 \
             WHERE id=?6",
            params![
                rel.name,
                rel.from_table,
                rel.to_table,
                rel.from_field,
                rel.to_field,
                id
            ],
        )?;
        if let Some(old_source) = &old_source {
            stamp_reference(&tx, old_source, None)?;
        }
        stamp_reference(&tx, &source, Some(&saved))?;
        tx.commit()?;
        self.relationship_by_id(id)
    }

    /// Delete a declared relationship by id, clearing the source field's options
    /// `reference` key in the same transaction.
    pub fn delete_relationship(&mut self, id: i64) -> Result<usize> {
        let Some(old) = self.relationship_by_id(id)? else {
            return Ok(0);
        };
        let source = self.field_by_id(old.from_table, old.from_field)?;
        let tx = self.app.transaction()?;
        let n = tx.execute("DELETE FROM meta_relationship WHERE id=?1", params![id])?;
        if let Some(source) = &source {
            stamp_reference(&tx, source, None)?;
        }
        tx.commit()?;
        Ok(n)
    }

    /// Atomically align a field's reference constraint with the relationship
    /// store (#128 theme D): upsert the field's relationship row when `reference`
    /// is `Some` (first existing relationship from the field wins, matching the
    /// schema surface's read side), or delete it when `None` — each leg keeps
    /// the options `reference` key in step within one transaction, so a
    /// mid-sequence failure cannot leave the two sides disagreeing.
    ///
    /// Returns the saved relationship (`None` when the reference was cleared).
    /// Fails with a downcastable [`crate::FieldReferenceError`] when the source
    /// or target field does not exist.
    pub fn set_field_reference(
        &mut self,
        table_id: i64,
        field_id: i64,
        reference: Option<&FieldReference>,
    ) -> Result<Option<RelationshipMeta>> {
        if self.field_by_id(table_id, field_id)?.is_none() {
            bail!(FieldReferenceError::SourceFieldMissing);
        }
        let existing = self
            .relationships_from_table(table_id)?
            .into_iter()
            .find(|r| r.from_field == field_id);
        let Some(reference) = reference else {
            if let Some(rel) = existing {
                self.delete_relationship(rel.id)?;
            }
            return Ok(None);
        };
        if self
            .field_by_id(reference.to_table, reference.to_field)?
            .is_none()
        {
            bail!(FieldReferenceError::TargetFieldMissing);
        }
        let rel = NewRelationship {
            name: reference.name.clone(),
            from_table: table_id,
            to_table: reference.to_table,
            from_field: field_id,
            to_field: reference.to_field,
        };
        let saved = match existing {
            Some(existing) => self.update_relationship(existing.id, &rel)?,
            None => self.create_relationship(&rel)?,
        };
        match saved {
            Some(rel) => Ok(Some(rel)),
            None => bail!(FieldReferenceError::RelationshipFieldsMissing),
        }
    }

    fn relationship_refs_are_valid(&self, rel: &NewRelationship) -> Result<bool> {
        Ok(self.table_by_id(rel.from_table)?.is_some()
            && self.table_by_id(rel.to_table)?.is_some()
            && self.field_by_id(rel.from_table, rel.from_field)?.is_some()
            && self.field_by_id(rel.to_table, rel.to_field)?.is_some())
    }

    fn rebuild_physical_table(&mut self, table: &TableMeta, fields: &[FieldMeta]) -> Result<()> {
        let tmp = format!("{}_rm_rebuild", table.phys);
        let mut columns = vec!["id INTEGER PRIMARY KEY".to_string()];
        for field in fields {
            columns.push(format!("{} {}", field.phys, field.kind.sql_type()));
        }
        let names: Vec<&str> = fields.iter().map(|f| f.phys.as_str()).collect();
        let copy_columns = if names.is_empty() {
            "id".to_string()
        } else {
            format!("id, {}", names.join(", "))
        };
        let tx = self.data.transaction()?;
        tx.execute(&format!("DROP TABLE IF EXISTS {tmp}"), [])?;
        tx.execute(&format!("CREATE TABLE {tmp} ({})", columns.join(", ")), [])?;
        tx.execute(
            &format!(
                "INSERT INTO {tmp} ({copy_columns}) SELECT {copy_columns} FROM {}",
                table.phys
            ),
            [],
        )?;
        tx.execute(&format!("DROP TABLE {}", table.phys), [])?;
        tx.execute(&format!("ALTER TABLE {tmp} RENAME TO {}", table.phys), [])?;
        tx.commit()?;
        Ok(())
    }
}

/// Rewrite `field`'s options `reference` key inside `tx` — set from `rel`, or
/// removed when `None`. The write shares the relationship row's transaction so
/// both sides commit (or roll back) together.
fn stamp_reference(
    tx: &rusqlite::Transaction<'_>,
    field: &FieldMeta,
    rel: Option<&RelationshipMeta>,
) -> Result<()> {
    let options = field.options_value();
    let options = match rel {
        Some(rel) => crate::options::with_reference(options, rel),
        None => crate::options::without_reference(options),
    };
    tx.execute(
        "UPDATE meta_field SET options=?1 WHERE id=?2",
        params![serde_json::to_string(&options)?, field.id],
    )?;
    Ok(())
}

fn relationship_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RelationshipMeta> {
    Ok(RelationshipMeta {
        id: row.get(0)?,
        name: row.get(1)?,
        from_table: row.get(2)?,
        to_table: row.get(3)?,
        from_field: row.get(4)?,
        to_field: row.get(5)?,
    })
}

fn value_list_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ValueListMeta> {
    Ok(ValueListMeta {
        id: row.get(0)?,
        name: row.get(1)?,
        source: row.get(2)?,
        config: row.get(3)?,
        position: row.get(4)?,
    })
}

fn validate_value_list_source(source: &str) -> Result<()> {
    match source {
        "custom" | "field" => Ok(()),
        other => bail!("unsupported value-list source {other}"),
    }
}

fn validate_value_list_config(source: &str, config: &str) -> Result<()> {
    let parsed: Value = serde_json::from_str(config).context("parse value-list config")?;
    match source {
        "custom" => {
            if !parsed.get("values").is_some_and(|v| {
                v.as_array()
                    .is_some_and(|items| items.iter().all(Value::is_string))
            }) {
                bail!("custom value list needs string values");
            }
        }
        "field" => {
            if parsed.get("fromField").and_then(Value::as_i64).is_none() {
                bail!("field value list needs fromField");
            }
            if let Some(sort) = parsed.get("sort").and_then(Value::as_str) {
                if sort != "first" && sort != "second" {
                    bail!("field value list sort must be first or second");
                }
            }
        }
        other => bail!("unsupported value-list source {other}"),
    }
    Ok(())
}

fn resolve_custom_value_list(config: &str) -> Result<Vec<ValueListItem>> {
    let parsed: Value = serde_json::from_str(config).context("parse custom value-list config")?;
    let values = parsed
        .get("values")
        .and_then(Value::as_array)
        .context("custom value list needs values")?;
    Ok(values
        .iter()
        .filter_map(Value::as_str)
        .map(|value| {
            let divider = value == "-";
            ValueListItem {
                value: if divider {
                    String::new()
                } else {
                    value.to_string()
                },
                display: value.to_string(),
                divider,
            }
        })
        .collect())
}

fn value_list_name_exists(sol: &Solution, name: &str) -> Result<bool> {
    let mut stmt = sol
        .app
        .prepare("SELECT 1 FROM meta_value_list WHERE name=?1 LIMIT 1")?;
    let mut rows = stmt.query(params![name])?;
    Ok(rows.next()?.is_some())
}

fn value_ref_to_string(v: ValueRef<'_>) -> String {
    match v {
        ValueRef::Null => String::new(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
        ValueRef::Blob(_) => "<blob>".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{FieldKind, NewField, NewRelationship, NewValueList, Solution, ValueListItem};

    #[test]
    fn field_kind_str_parse_and_sql_type_round_trip() {
        for kind in [
            FieldKind::Text,
            FieldKind::Number,
            FieldKind::Date,
            FieldKind::Bool,
            FieldKind::Time,
            FieldKind::Timestamp,
        ] {
            assert_eq!(FieldKind::parse(kind.as_str()), Some(kind));
        }

        assert_eq!(FieldKind::Time.as_str(), "time");
        assert_eq!(FieldKind::Timestamp.as_str(), "timestamp");
        // Both temporal kinds land on TEXT affinity (ISO-8601 storage contract).
        assert_eq!(FieldKind::Time.sql_type(), "TEXT");
        assert_eq!(FieldKind::Timestamp.sql_type(), "TEXT");
        assert_eq!(FieldKind::parse("bogus"), None);
    }

    #[test]
    fn create_table_generates_default_form_layout() {
        let mut s = Solution::open_in_memory().unwrap();
        let tid = s
            .create_table(
                "Invoices",
                &[
                    NewField {
                        name: "Number".into(),
                        kind: FieldKind::Text,
                    },
                    NewField {
                        name: "Total".into(),
                        kind: FieldKind::Number,
                    },
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
        assert!(layouts
            .iter()
            .all(|l| l.name == "Invoices" && l.table_id == tid));
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
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
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
        assert_eq!(
            (rows[0].0.as_str(), rows[0].5.as_deref()),
            ("text", Some("Number"))
        );
        assert_eq!(
            (rows[1].0.as_str(), rows[1].4.as_deref()),
            ("field", Some("Invoices.Number"))
        );
        assert_eq!(
            (rows[2].0.as_str(), rows[2].5.as_deref()),
            ("text", Some("Total"))
        );
        assert_eq!(
            (rows[3].0.as_str(), rows[3].4.as_deref()),
            ("field", Some("Invoices.Total"))
        );
        assert!(
            rows[0].1 == rows[1].1 && rows[2].1 == rows[3].1,
            "label shares its value's row"
        );
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

    #[test]
    fn rename_table_and_field_keep_direct_bindings_resolvable() {
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
        let fid = s.fields(tid).unwrap()[0].id;

        s.rename_table(tid, "People").unwrap().unwrap();
        s.rename_field(tid, fid, "Full Name").unwrap().unwrap();

        let bindings: Vec<String> = {
            let mut stmt = s
                .app
                .prepare("SELECT binding FROM meta_object WHERE binding IS NOT NULL")
                .unwrap();
            stmt.query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert!(bindings.iter().all(|b| b == "People.Full Name"));
    }

    #[test]
    fn field_retype_reorder_and_delete_update_metadata_and_data_table() {
        let mut s = Solution::open_in_memory().unwrap();
        let tid = s
            .create_table(
                "Things",
                &[
                    NewField {
                        name: "Name".into(),
                        kind: FieldKind::Text,
                    },
                    NewField {
                        name: "Score".into(),
                        kind: FieldKind::Number,
                    },
                ],
            )
            .unwrap();
        let table = s.table_by_id(tid).unwrap().unwrap();
        let fields = s.fields(tid).unwrap();
        s.insert_record(
            &table,
            &[(&fields[0], "A".into()), (&fields[1], "7".into())],
        )
        .unwrap();

        s.retype_field(tid, fields[1].id, FieldKind::Text)
            .unwrap()
            .unwrap();
        let columns: Vec<(String, String)> = {
            let mut stmt = s
                .data
                .prepare(&format!("PRAGMA table_info({})", table.phys))
                .unwrap();
            stmt.query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert_eq!(
            columns
                .iter()
                .find(|(name, _)| name == &fields[1].phys)
                .unwrap()
                .1,
            "TEXT"
        );
        let cells = &s.list_records(&table, &s.fields(tid).unwrap()).unwrap()[0].cells;
        assert_eq!(cells[0], "A");
        assert_eq!(cells[1].parse::<f64>().unwrap(), 7.0);

        let reordered = s
            .reorder_fields(tid, &[fields[1].id, fields[0].id])
            .unwrap();
        assert_eq!(
            reordered
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Score", "Name"]
        );

        assert_eq!(s.delete_field(tid, fields[0].id).unwrap(), 1);
        let after = s.fields(tid).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].name, "Score");
        assert_eq!(
            s.list_records(&table, &after).unwrap()[0].cells[0]
                .parse::<f64>()
                .unwrap(),
            7.0
        );
    }

    #[test]
    fn value_list_crud_and_custom_resolution() {
        let mut s = Solution::open_in_memory().unwrap();
        let list = s
            .create_value_list(&NewValueList {
                name: "Sizes".into(),
                source: "custom".into(),
                config: r#"{"values":["Small","Medium","-","Large"]}"#.into(),
            })
            .unwrap();
        assert_eq!(list.name, "Sizes");
        assert_eq!(s.value_lists().unwrap().len(), 1);
        assert_eq!(
            s.resolve_value_list(list.id).unwrap().unwrap(),
            vec![
                ValueListItem {
                    value: "Small".into(),
                    display: "Small".into(),
                    divider: false,
                },
                ValueListItem {
                    value: "Medium".into(),
                    display: "Medium".into(),
                    divider: false,
                },
                ValueListItem {
                    value: String::new(),
                    display: "-".into(),
                    divider: true,
                },
                ValueListItem {
                    value: "Large".into(),
                    display: "Large".into(),
                    divider: false,
                },
            ]
        );

        let updated = s
            .update_value_list(
                list.id,
                &NewValueList {
                    name: "Sizes v2".into(),
                    source: "custom".into(),
                    config: r#"{"values":["One"]}"#.into(),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(updated.name, "Sizes v2");
        let copy = s.duplicate_value_list(updated.id, None).unwrap().unwrap();
        assert_eq!(copy.name, "Sizes v2 Copy");
        assert_eq!(s.delete_value_list(updated.id).unwrap(), 1);
        assert!(s.value_list_by_id(updated.id).unwrap().is_none());
    }

    #[test]
    fn field_value_list_resolves_distinct_values() {
        let mut s = Solution::open_in_memory().unwrap();
        let tid = s
            .create_table(
                "Products",
                &[
                    NewField {
                        name: "Code".into(),
                        kind: FieldKind::Text,
                    },
                    NewField {
                        name: "Name".into(),
                        kind: FieldKind::Text,
                    },
                ],
            )
            .unwrap();
        let table = s.table_by_id(tid).unwrap().unwrap();
        let fields = s.fields(tid).unwrap();
        s.insert_record(
            &table,
            &[(&fields[0], "B".into()), (&fields[1], "Beta".into())],
        )
        .unwrap();
        s.insert_record(
            &table,
            &[(&fields[0], "A".into()), (&fields[1], "Alpha".into())],
        )
        .unwrap();
        s.insert_record(
            &table,
            &[(&fields[0], "B".into()), (&fields[1], "Beta".into())],
        )
        .unwrap();

        let list = s
            .create_value_list(&NewValueList {
                name: "Products".into(),
                source: "field".into(),
                config: format!(
                    r#"{{"fromField":{},"secondField":{},"showSecondOnly":true,"sort":"second"}}"#,
                    fields[0].id, fields[1].id
                ),
            })
            .unwrap();
        let items = s.resolve_value_list(list.id).unwrap().unwrap();
        assert_eq!(
            items,
            vec![
                ValueListItem {
                    value: "A".into(),
                    display: "Alpha".into(),
                    divider: false,
                },
                ValueListItem {
                    value: "B".into(),
                    display: "Beta".into(),
                    divider: false,
                },
            ]
        );
    }

    #[test]
    fn relationship_crud_validates_table_field_ownership() {
        let mut s = Solution::open_in_memory().unwrap();
        let customers = s
            .create_table(
                "Customers",
                &[NewField {
                    name: "Id".into(),
                    kind: FieldKind::Number,
                }],
            )
            .unwrap();
        let invoices = s
            .create_table(
                "Invoices",
                &[NewField {
                    name: "Customer Id".into(),
                    kind: FieldKind::Number,
                }],
            )
            .unwrap();
        let customer_id = s.fields(customers).unwrap()[0].id;
        let invoice_customer_id = s.fields(invoices).unwrap()[0].id;

        let rel = s
            .create_relationship(&NewRelationship {
                name: "customer".into(),
                from_table: invoices,
                to_table: customers,
                from_field: invoice_customer_id,
                to_field: customer_id,
            })
            .unwrap()
            .unwrap();
        assert_eq!(rel.name, "customer");
        assert_eq!(s.relationships_from_table(invoices).unwrap().len(), 1);

        assert!(s
            .create_relationship(&NewRelationship {
                name: "bad".into(),
                from_table: invoices,
                to_table: customers,
                from_field: customer_id,
                to_field: invoice_customer_id,
            })
            .unwrap()
            .is_none());

        let renamed = s
            .update_relationship(
                rel.id,
                &NewRelationship {
                    name: "bill_to".into(),
                    from_table: invoices,
                    to_table: customers,
                    from_field: invoice_customer_id,
                    to_field: customer_id,
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(renamed.name, "bill_to");
        assert_eq!(s.delete_relationship(rel.id).unwrap(), 1);
        assert!(s.relationships().unwrap().is_empty());
    }

    #[test]
    fn reorder_tables_persists_order_and_validates_set() {
        let mut s = Solution::open_in_memory().unwrap();
        let a = s.create_table("A", &[]).unwrap();
        let b = s.create_table("B", &[]).unwrap();
        let c = s.create_table("C", &[]).unwrap();

        let initial = s.tables().unwrap();
        assert_eq!(initial[0].id, a);
        assert_eq!(initial[1].id, b);
        assert_eq!(initial[2].id, c);

        let reordered = s.reorder_tables(&[c, a, b]).unwrap();
        assert_eq!(reordered[0].id, c);
        assert_eq!(reordered[1].id, a);
        assert_eq!(reordered[2].id, b);

        // Fetch again to verify persistence
        let fetched = s.tables().unwrap();
        assert_eq!(fetched[0].id, c);
        assert_eq!(fetched[1].id, a);
        assert_eq!(fetched[2].id, b);

        // Validation checks
        assert!(s.reorder_tables(&[a]).is_err());
        assert!(s.reorder_tables(&[a, a, b]).is_err());
        assert!(s.reorder_tables(&[a, b, 9999]).is_err());
    }
}
