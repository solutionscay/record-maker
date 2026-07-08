//! #10 — generic, metadata-driven record CRUD over data.db.
//!
//! There is no per-table code: every query is built at runtime from a table's
//! field metadata. Physical identifiers (`t_<id>`/`f_<id>`) are id-derived and
//! therefore safe to interpolate; all user *values* are bound as parameters.

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::types::{Value, ValueRef};
use rusqlite::{params, params_from_iter};

use crate::model::{FieldMeta, TableMeta};
use crate::options::FieldOptions;
use crate::Solution;

/// A row, with `cells` aligned to the field order passed to [`Solution::list_records`].
#[derive(Debug, Clone)]
pub struct Record {
    pub id: i64,
    pub cells: Vec<String>,
}

impl Solution {
    /// List all rows of `table`, with cells aligned to `fields`.
    pub fn list_records(&self, table: &TableMeta, fields: &[FieldMeta]) -> Result<Vec<Record>> {
        let mut select = String::from("id");
        for f in fields {
            select.push_str(", ");
            select.push_str(&f.phys);
        }
        let sql = format!("SELECT {select} FROM {} ORDER BY id", table.phys);
        let mut stmt = self.data.prepare(&sql)?;
        let n = fields.len();
        let rows = stmt.query_map([], |row| {
            let id: i64 = row.get(0)?;
            let mut cells = Vec::with_capacity(n);
            for i in 0..n {
                cells.push(value_to_string(row.get_ref(i + 1)?));
            }
            Ok(Record { id, cells })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The **found set** of `table`: the ids of all its rows, ordered by id.
    ///
    /// This is the record-navigation primitive (#23). Until Find exists the
    /// found set is every record (`found == total`); the flipbook navigates
    /// 1-based positions within this list, and `len()` is the "of M" total.
    /// Find will later return a filtered version of exactly this shape.
    pub fn record_ids(&self, table: &TableMeta) -> Result<Vec<i64>> {
        let mut stmt = self
            .data
            .prepare(&format!("SELECT id FROM {} ORDER BY id", table.phys))?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The found-set size of `table` — `record_ids().len()` without loading the
    /// ids. Pairs with [`Solution::record_id_at`] so a single-record lookup
    /// (clamp + index) never materialises the whole found set.
    pub fn record_count(&self, table: &TableMeta) -> Result<i64> {
        let n = self.data.query_row(
            &format!("SELECT COUNT(*) FROM {}", table.phys),
            [],
            |r| r.get::<_, i64>(0),
        )?;
        Ok(n)
    }

    /// The physical id at 1-based found-set position `pos` (id order — the same
    /// ordering as [`Solution::record_ids`]), or `None` when out of range.
    pub fn record_id_at(&self, table: &TableMeta, pos: i64) -> Result<Option<i64>> {
        if pos < 1 {
            return Ok(None);
        }
        let mut stmt = self.data.prepare(&format!(
            "SELECT id FROM {} ORDER BY id LIMIT 1 OFFSET ?1",
            table.phys
        ))?;
        let mut rows = stmt.query_map(params![pos - 1], |r| r.get::<_, i64>(0))?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Insert a row. `values` are `(field, string-value)` pairs; SQLite type
    /// affinity converts the strings into each column's type.
    ///
    /// Every field's validation rules (required / unique / range /
    /// member-of-value-list — see [`crate::options`]) are enforced first; a
    /// rejected write fails with a downcastable [`crate::ValidationError`].
    pub fn insert_record(&self, table: &TableMeta, values: &[(&FieldMeta, String)]) -> Result<i64> {
        // A Primary ID field is system-managed: auto-assign a UUID to any primary
        // field left empty/absent on insert, so the key is never typed and its
        // implied required + unique rules are satisfied automatically. Only on
        // insert — a key is assigned once at creation, never regenerated.
        let all_fields = self.fields(table.id)?;
        let mut provided: HashMap<i64, String> =
            values.iter().map(|(f, v)| (f.id, v.clone())).collect();
        for field in &all_fields {
            let is_primary = FieldOptions::parse(&field.options)
                .validation
                .map(|r| r.primary)
                .unwrap_or(false);
            let empty = provided
                .get(&field.id)
                .map(|v| v.trim().is_empty())
                .unwrap_or(true);
            if is_primary && empty {
                provided.insert(field.id, self.generate_uuid()?);
            }
        }
        // Rebuild in field order, keeping every column that was provided or
        // auto-filled. Field refs come from `all_fields` (owned here).
        let augmented: Vec<(&FieldMeta, String)> = all_fields
            .iter()
            .filter_map(|f| provided.get(&f.id).map(|v| (f, v.clone())))
            .collect();

        self.validate_record_values(table, &augmented, None)?;
        if augmented.is_empty() {
            self.data
                .execute(&format!("INSERT INTO {} DEFAULT VALUES", table.phys), [])?;
            return Ok(self.data.last_insert_rowid());
        }
        let cols: Vec<&str> = augmented.iter().map(|(f, _)| f.phys.as_str()).collect();
        let marks: Vec<String> = (1..=augmented.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            table.phys,
            cols.join(", "),
            marks.join(", ")
        );
        self.data.execute(
            &sql,
            params_from_iter(augmented.iter().map(|(_, v)| v.clone())),
        )?;
        Ok(self.data.last_insert_rowid())
    }

    /// A random v4 UUID, sourced from SQLite's `randomblob` so the engine needs
    /// no extra crate. Used to auto-assign Primary ID field values on insert.
    fn generate_uuid(&self) -> Result<String> {
        let hex: String =
            self.data
                .query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?;
        let mut b = [0u8; 16];
        for (i, byte) in b.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0);
        }
        b[6] = (b[6] & 0x0f) | 0x40; // version 4
        b[8] = (b[8] & 0x3f) | 0x80; // RFC 4122 variant
        Ok(format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13],
            b[14], b[15]
        ))
    }

    /// Delete a row by its physical id.
    pub fn delete_record(&self, table: &TableMeta, row_id: i64) -> Result<()> {
        self.data.execute(
            &format!("DELETE FROM {} WHERE id=?1", table.phys),
            params![row_id],
        )?;
        Ok(())
    }

    /// Fetch a single row's cells aligned to `fields`. `None` if no such id.
    pub fn get_record(
        &self,
        table: &TableMeta,
        fields: &[FieldMeta],
        id: i64,
    ) -> Result<Option<Vec<String>>> {
        let mut select = String::from("id");
        for f in fields {
            select.push_str(", ");
            select.push_str(&f.phys);
        }
        let sql = format!("SELECT {select} FROM {} WHERE id=?1", table.phys);
        let mut stmt = self.data.prepare(&sql)?;
        let n = fields.len();
        let mut rows = stmt.query_map(params![id], |row| {
            let mut cells = Vec::with_capacity(n);
            for i in 0..n {
                cells.push(value_to_string(row.get_ref(i + 1)?));
            }
            Ok(cells)
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Update a row's fields by id. No-op if `values` is empty.
    ///
    /// Validation runs first (excluding row `id` from uniqueness); a rejected
    /// write fails with a downcastable [`crate::ValidationError`]. Note that a
    /// required field missing from `values` fails validation even though the
    /// write would not have touched it — commits submit the full record.
    pub fn update_record(
        &self,
        table: &TableMeta,
        id: i64,
        values: &[(&FieldMeta, String)],
    ) -> Result<()> {
        self.validate_record_values(table, values, Some(id))?;
        if values.is_empty() {
            return Ok(());
        }
        let sets: Vec<String> = values
            .iter()
            .enumerate()
            .map(|(i, (f, _))| format!("{}=?{}", f.phys, i + 1))
            .collect();
        let sql = format!(
            "UPDATE {} SET {} WHERE id=?{}",
            table.phys,
            sets.join(", "),
            values.len() + 1
        );
        let mut ps: Vec<Value> = values.iter().map(|(_, v)| Value::Text(v.clone())).collect();
        ps.push(Value::Integer(id));
        self.data.execute(&sql, params_from_iter(ps))?;
        Ok(())
    }

    /// True when a field already contains `value`, optionally excluding one row.
    /// Used by schema-level validation before writes reach SQLite.
    pub fn field_value_exists(
        &self,
        table: &TableMeta,
        field: &FieldMeta,
        value: &str,
        exclude_id: Option<i64>,
    ) -> Result<bool> {
        let (sql, params): (String, Vec<Value>) = match exclude_id {
            Some(id) => (
                format!(
                    "SELECT 1 FROM {} WHERE {}=?1 AND id<>?2 LIMIT 1",
                    table.phys, field.phys
                ),
                vec![Value::Text(value.to_string()), Value::Integer(id)],
            ),
            None => (
                format!("SELECT 1 FROM {} WHERE {}=?1 LIMIT 1", table.phys, field.phys),
                vec![Value::Text(value.to_string())],
            ),
        };
        let mut stmt = self.data.prepare(&sql)?;
        let mut rows = stmt.query(params_from_iter(params))?;
        Ok(rows.next()?.is_some())
    }
}

fn value_to_string(v: ValueRef<'_>) -> String {
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
    use crate::{FieldKind, NewField, Solution};

    #[test]
    fn create_table_then_crud_roundtrip() {
        let mut s = Solution::open_in_memory().unwrap();
        let tid = s
            .create_table(
                "Customers",
                &[
                    NewField { name: "Name".into(), kind: FieldKind::Text },
                    NewField { name: "Age".into(), kind: FieldKind::Number },
                ],
            )
            .unwrap();

        let table = s.table_by_name("Customers").unwrap().unwrap();
        assert_eq!(table.id, tid);
        let fields = s.fields(tid).unwrap();
        assert_eq!(fields.len(), 2);

        // the physical table really exists in data.db
        let exists: i64 = s
            .data
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [&table.phys],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(exists, 1);

        let row = |name: &str, age: &str| {
            vec![
                (&fields[0], name.to_string()),
                (&fields[1], age.to_string()),
            ]
        };
        s.insert_record(&table, &row("Ada", "36")).unwrap();
        s.insert_record(&table, &row("Linus", "54")).unwrap();

        let recs = s.list_records(&table, &fields).unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].cells[0], "Ada");
        assert_eq!(recs[0].cells[1].parse::<f64>().unwrap(), 36.0);

        s.delete_record(&table, recs[0].id).unwrap();
        let recs = s.list_records(&table, &fields).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].cells[0], "Linus");
    }

    #[test]
    fn primary_key_autofills_uuid_on_insert() {
        let mut s = Solution::open_in_memory().unwrap();
        let tid = s
            .create_table(
                "Customers",
                &[
                    NewField { name: "Customer_ID".into(), kind: FieldKind::Text },
                    NewField { name: "Name".into(), kind: FieldKind::Text },
                ],
            )
            .unwrap();
        let table = s.table_by_id(tid).unwrap().unwrap();
        let fields = s.fields(tid).unwrap();
        s.update_field_options(tid, fields[0].id, r#"{"validation":{"primary":true}}"#)
            .unwrap();
        let fields = s.fields(tid).unwrap();

        // A blank New record (no submitted values) must succeed now: the empty
        // primary auto-fills with a v4 UUID, satisfying its implied required+unique.
        let id = s.insert_record(&table, &[]).unwrap();
        let key = s.get_record(&table, &fields, id).unwrap().unwrap()[0].clone();
        assert_eq!(key.len(), 36, "primary auto-filled a UUID, got {key:?}");
        assert_eq!(key.as_bytes()[14], b'4', "version nibble");
        assert!(matches!(key.as_bytes()[19], b'8' | b'9' | b'a' | b'b'), "variant nibble");

        // Distinct keys per record.
        let id2 = s.insert_record(&table, &[(&fields[1], "Bob".into())]).unwrap();
        let key2 = s.get_record(&table, &fields, id2).unwrap().unwrap()[0].clone();
        assert_ne!(key2, key);
        assert!(!key2.is_empty());

        // An explicitly supplied primary value is respected, not overwritten.
        let id3 = s.insert_record(&table, &[(&fields[0], "CUST-1".into())]).unwrap();
        let rec3 = s.get_record(&table, &fields, id3).unwrap().unwrap();
        assert_eq!(rec3[0], "CUST-1");
    }

    #[test]
    fn record_ids_is_the_ordered_found_set() {
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table("T", &[NewField { name: "N".into(), kind: FieldKind::Text }])
            .unwrap();
        let table = s.table_by_name("T").unwrap().unwrap();
        let fields = s.fields(table.id).unwrap();

        assert!(s.record_ids(&table).unwrap().is_empty());

        let a = s.insert_record(&table, &[(&fields[0], "a".into())]).unwrap();
        let b = s.insert_record(&table, &[(&fields[0], "b".into())]).unwrap();
        let c = s.insert_record(&table, &[(&fields[0], "c".into())]).unwrap();
        assert_eq!(s.record_ids(&table).unwrap(), vec![a, b, c]);

        // deletion shrinks the found set; order is preserved
        s.delete_record(&table, b).unwrap();
        assert_eq!(s.record_ids(&table).unwrap(), vec![a, c]);
    }

    #[test]
    fn get_and_update_record() {
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table(
            "People",
            &[
                NewField { name: "Name".into(), kind: FieldKind::Text },
                NewField { name: "Age".into(), kind: FieldKind::Number },
            ],
        )
        .unwrap();
        let table = s.table_by_name("People").unwrap().unwrap();
        let fields = s.fields(table.id).unwrap();
        let id = s
            .insert_record(&table, &[(&fields[0], "Ada".into()), (&fields[1], "36".into())])
            .unwrap();

        let got = s.get_record(&table, &fields, id).unwrap().unwrap();
        assert_eq!(got[0], "Ada");
        assert!(s.get_record(&table, &fields, 999_999).unwrap().is_none());

        s.update_record(
            &table,
            id,
            &[(&fields[0], "Ada L".into()), (&fields[1], "37".into())],
        )
        .unwrap();
        let got2 = s.get_record(&table, &fields, id).unwrap().unwrap();
        assert_eq!(got2[0], "Ada L");
        assert_eq!(got2[1].parse::<f64>().unwrap(), 37.0);
    }
}
