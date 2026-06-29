//! #10 — generic, metadata-driven record CRUD over data.db.
//!
//! There is no per-table code: every query is built at runtime from a table's
//! field metadata. Physical identifiers (`t_<id>`/`f_<id>`) are id-derived and
//! therefore safe to interpolate; all user *values* are bound as parameters.

use anyhow::Result;
use rusqlite::types::ValueRef;
use rusqlite::{params, params_from_iter};

use crate::model::{FieldMeta, TableMeta};
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

    /// Insert a row. `values` are `(field, string-value)` pairs; SQLite type
    /// affinity converts the strings into each column's type.
    pub fn insert_record(&self, table: &TableMeta, values: &[(&FieldMeta, String)]) -> Result<i64> {
        if values.is_empty() {
            self.data
                .execute(&format!("INSERT INTO {} DEFAULT VALUES", table.phys), [])?;
            return Ok(self.data.last_insert_rowid());
        }
        let cols: Vec<&str> = values.iter().map(|(f, _)| f.phys.as_str()).collect();
        let marks: Vec<String> = (1..=values.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            table.phys,
            cols.join(", "),
            marks.join(", ")
        );
        self.data.execute(
            &sql,
            params_from_iter(values.iter().map(|(_, v)| v.clone())),
        )?;
        Ok(self.data.last_insert_rowid())
    }

    /// Delete a row by its physical id.
    pub fn delete_record(&self, table: &TableMeta, row_id: i64) -> Result<()> {
        self.data.execute(
            &format!("DELETE FROM {} WHERE id=?1", table.phys),
            params![row_id],
        )?;
        Ok(())
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
}
