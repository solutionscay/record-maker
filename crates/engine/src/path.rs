//! #11 — the relationship path resolver (the #108 linchpin).
//!
//! Resolves a dot-path binding (`Invoice.customer.name`) over the **named
//! relationship graph** (ADR-0003: no table occurrences) into a concrete
//! related-record set, and classifies the route by the *determined-stamp rule*
//! that #111's related-record CRUD engine consumes.
//!
//! A route is a chain of hops. Each hop traverses one declared FK relationship
//! in one of two directions:
//!
//! * **forward** — the current table owns the FK (`from_table`); following it
//!   yields at most one parent record (to-one).
//! * **reverse** — the current table is the referenced parent (`to_table`);
//!   following it yields the set of children that point back (to-many).
//!
//! One relationship declaration, both directions ([`RelationshipMeta`]).
//!
//! ## Determined-stamp rule ([`RouteClass`])
//!
//! Create must stamp every FK that makes the new row *belong*; a route is
//! **create-determined** only when every such FK resolves to a single, known
//! record:
//!
//! * **direct FK** (one hop) — stamp the one FK with the base record's key.
//! * **join-table M:N** (`A → to-many join → to-one terminal`) — insert a join
//!   row stamping the base key and the selected terminal key; both determined.
//!
//! Any other multi-hop shape that crosses a to-many boundary leaves the
//! immediate parent undetermined, so create cannot be inverted — it is refused.
//! Read/update/delete act on a real terminal record and are always available;
//! this module only owns *resolution + classification*.

use std::fmt;

use anyhow::{bail, Result};
use rusqlite::params_from_iter;
use rusqlite::types::{Value, ValueRef};

use crate::model::{Cardinality, FieldMeta, RelationshipMeta, TableMeta};
use crate::Solution;

/// Which way a hop traverses its relationship declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HopDirection {
    /// The current table owns the FK (`from_table` → `to_table`): to-one.
    Forward,
    /// The current table is the referenced parent (`to_table` → `from_table`):
    /// to-many.
    Reverse,
}

/// One traversed relationship in a resolved route.
#[derive(Debug, Clone)]
pub struct RouteHop {
    /// The declared relationship this hop follows.
    pub relationship_id: i64,
    /// Direction traversed.
    pub direction: HopDirection,
    /// Cardinality of this hop (to-one forward / to-many reverse).
    pub cardinality: Cardinality,
    /// The table this hop lands on (its record set after the hop).
    pub result_table: i64,
}

/// How a route classifies under the determined-stamp rule (#111 consumes this).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteClass {
    /// No relationship hops — the path addresses the base record's own field.
    /// There is no *related* record to create through it.
    BaseRecord,
    /// A single FK hop. Create stamps the one FK with the base key. Determined.
    DirectFk,
    /// `A → to-many join → to-one terminal`: M:N through a join table. Create
    /// inserts a join row stamping the base key + the selected terminal key.
    /// Determined.
    JoinTableManyToMany,
    /// A multi-hop route that crosses a to-many boundary such that the immediate
    /// create-parent is undetermined. Read/update/delete still resolve; create
    /// is refused.
    Undetermined,
}

impl RouteClass {
    /// Whether the determined-stamp rule holds — i.e. #111 may offer create
    /// through this route. Only a direct FK or a join-table M:N qualify.
    pub const fn create_determined(self) -> bool {
        matches!(self, RouteClass::DirectFk | RouteClass::JoinTableManyToMany)
    }
}

/// A parsed + classified dot-path route through the relationship graph.
///
/// Produced by [`Solution::resolve_path`]; walked to a concrete record set by
/// [`Solution::route_record_set`].
#[derive(Debug, Clone)]
pub struct ResolvedRoute {
    /// The layout's base/primary table the path is rooted at.
    pub base_table: i64,
    /// Relationship hops in traversal order (empty for a base-record field).
    pub hops: Vec<RouteHop>,
    /// The table the resolved record set belongs to (the last hop's landing
    /// table, or the base table when there are no hops).
    pub terminal_table: i64,
    /// The trailing field segment (`…​.name`), if the path binds a value rather
    /// than stopping at a record set.
    pub terminal_field: Option<i64>,
    /// The determined-stamp classification.
    pub class: RouteClass,
}

/// Why a dot-path could not be resolved against the relationship graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    /// The path was empty (no segments).
    Empty,
    /// The base table id does not exist.
    UnknownBaseTable,
    /// A segment matched neither a relationship from the current table nor a
    /// field on it.
    UnknownSegment(String),
    /// A field segment appeared before the end of the path (a field is only
    /// ever the terminal value binding).
    FieldNotTerminal(String),
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("empty path"),
            Self::UnknownBaseTable => f.write_str("unknown base table"),
            Self::UnknownSegment(s) => write!(f, "unknown path segment '{s}'"),
            Self::FieldNotTerminal(s) => {
                write!(f, "field '{s}' is not the final path segment")
            }
        }
    }
}

impl std::error::Error for PathError {}

/// Classify a hop chain by the determined-stamp rule. Only the relationship
/// cardinalities matter; the trailing field binding never changes the class.
fn classify(hops: &[RouteHop]) -> RouteClass {
    match hops {
        [] => RouteClass::BaseRecord,
        [_] => RouteClass::DirectFk,
        [a, b]
            if a.cardinality == Cardinality::ToMany && b.cardinality == Cardinality::ToOne =>
        {
            RouteClass::JoinTableManyToMany
        }
        _ => RouteClass::Undetermined,
    }
}

impl Solution {
    /// Resolve a dot-path binding rooted at `base_table` into a classified
    /// [`ResolvedRoute`], following named relationships hop by hop.
    ///
    /// The leading segment may (but need not) name the base table itself —
    /// `Invoice.customer.name` and `customer.name` resolve identically against
    /// base table `Invoice`. Each remaining segment is matched, ambiguity
    /// resolved by relationship name (ADR-0003), as a relationship touching the
    /// current table — **forward** (the current table owns the FK) preferred,
    /// then **reverse**. A segment that is not a relationship must be a field on
    /// the current table, and only as the final value binding. All matching is
    /// ASCII-case-insensitive so stored casing need not be echoed exactly.
    ///
    /// Errors with a downcastable [`PathError`] when a segment resolves to
    /// nothing or a field appears mid-path.
    pub fn resolve_path(&self, base_table: i64, path: &str) -> Result<ResolvedRoute> {
        let segments: Vec<&str> = path
            .split('.')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if segments.is_empty() {
            bail!(PathError::Empty);
        }
        let base = match self.table_by_id(base_table)? {
            Some(t) => t,
            None => bail!(PathError::UnknownBaseTable),
        };

        // An optional leading base-table name is consumed, but only when it is
        // not itself a relationship from the base (a relationship name wins).
        let mut idx = 0;
        if segments[0].eq_ignore_ascii_case(&base.name)
            && self.match_hop(base_table, segments[0])?.is_none()
        {
            idx = 1;
        }

        let mut current = base_table;
        let mut hops: Vec<RouteHop> = Vec::new();
        let mut terminal_field: Option<i64> = None;

        while idx < segments.len() {
            let seg = segments[idx];
            if let Some(hop) = self.match_hop(current, seg)? {
                current = hop.result_table;
                hops.push(hop);
                idx += 1;
                continue;
            }
            if let Some(field) = self.field_by_name(current, seg)? {
                if idx != segments.len() - 1 {
                    bail!(PathError::FieldNotTerminal(seg.to_string()));
                }
                terminal_field = Some(field.id);
                idx += 1;
                continue;
            }
            bail!(PathError::UnknownSegment(seg.to_string()));
        }

        let class = classify(&hops);
        Ok(ResolvedRoute {
            base_table,
            terminal_table: current,
            hops,
            terminal_field,
            class,
        })
    }

    /// Walk a resolved route from a single base record (its physical rowid) to
    /// the concrete set of related record rowids in the terminal table, in id
    /// order. A base field route (no hops) yields `[base_id]` when the row
    /// exists.
    ///
    /// Relationships anchor on the system UUID primary key (#155): a forward hop
    /// reads the FK values off the current rows and gathers the parents whose
    /// key matches; a reverse hop reads the current rows' keys and gathers the
    /// children that point back. Null/blank keys never match.
    pub fn route_record_set(&self, route: &ResolvedRoute, base_id: i64) -> Result<Vec<i64>> {
        // Seed with the base row only if it actually exists.
        let base = self
            .table_by_id(route.base_table)?
            .ok_or(PathError::UnknownBaseTable)?;
        let exists: Option<i64> = self
            .data
            .query_row(
                &format!("SELECT id FROM {} WHERE id=?1", base.phys),
                [base_id],
                |r| r.get(0),
            )
            .ok();
        let mut current_ids: Vec<i64> = exists.into_iter().collect();

        for hop in &route.hops {
            if current_ids.is_empty() {
                break;
            }
            let rel = self
                .relationship_by_id(hop.relationship_id)?
                .ok_or_else(|| anyhow::anyhow!("relationship {} vanished", hop.relationship_id))?;
            let (src_tbl, src_field, dst_tbl, dst_field) = match hop.direction {
                // current owns the FK; match it against the parent's key.
                HopDirection::Forward => (
                    self.require_table(rel.from_table)?,
                    self.require_field(rel.from_table, rel.from_field)?,
                    self.require_table(rel.to_table)?,
                    self.require_field(rel.to_table, rel.to_field)?,
                ),
                // current is the parent; match its key against the children's FK.
                HopDirection::Reverse => (
                    self.require_table(rel.to_table)?,
                    self.require_field(rel.to_table, rel.to_field)?,
                    self.require_table(rel.from_table)?,
                    self.require_field(rel.from_table, rel.from_field)?,
                ),
            };
            let keys = self.column_values_for_rows(&src_tbl, &src_field, &current_ids)?;
            current_ids = self.row_ids_where_value_in(&dst_tbl, &dst_field, &keys)?;
        }
        Ok(current_ids)
    }

    /// Resolve a route to its bound values for one base record. When the route
    /// ends in a terminal field, returns that field's value for each record in
    /// the set (a to-one route yields at most one). With no terminal field, the
    /// records' system-managed rowids are returned as strings.
    pub fn route_values(&self, route: &ResolvedRoute, base_id: i64) -> Result<Vec<String>> {
        let ids = self.route_record_set(route, base_id)?;
        let Some(field_id) = route.terminal_field else {
            return Ok(ids.iter().map(i64::to_string).collect());
        };
        let table = self.require_table(route.terminal_table)?;
        let field = self.require_field(route.terminal_table, field_id)?;
        self.column_values_for_rows_all(&table, &field, &ids)
    }

    // --- resolution helpers ------------------------------------------------

    /// Match `segment` to a relationship touching `current` — forward (current
    /// owns the FK) preferred over reverse. `None` when no relationship matches.
    fn match_hop(&self, current: i64, segment: &str) -> Result<Option<RouteHop>> {
        let rels = self.relationships()?;
        // Forward: the segment names a relationship declared on `current`.
        if let Some(rel) = rels
            .iter()
            .find(|r| r.from_table == current && r.name.eq_ignore_ascii_case(segment))
        {
            return Ok(Some(self.hop(rel, HopDirection::Forward)));
        }
        // Reverse: the segment names a relationship whose parent is `current`.
        if let Some(rel) = rels
            .iter()
            .find(|r| r.to_table == current && r.name.eq_ignore_ascii_case(segment))
        {
            return Ok(Some(self.hop(rel, HopDirection::Reverse)));
        }
        Ok(None)
    }

    fn hop(&self, rel: &RelationshipMeta, direction: HopDirection) -> RouteHop {
        let (cardinality, result_table) = match direction {
            HopDirection::Forward => (rel.forward_cardinality(), rel.to_table),
            HopDirection::Reverse => (rel.reverse_cardinality(), rel.from_table),
        };
        RouteHop {
            relationship_id: rel.id,
            direction,
            cardinality,
            result_table,
        }
    }

    /// Find a field on `table_id` by name (ASCII-case-insensitive), including
    /// the system primary key.
    fn field_by_name(&self, table_id: i64, name: &str) -> Result<Option<FieldMeta>> {
        Ok(self
            .all_fields(table_id)?
            .into_iter()
            .find(|f| f.name.eq_ignore_ascii_case(name)))
    }

    fn require_table(&self, id: i64) -> Result<TableMeta> {
        self.table_by_id(id)?
            .ok_or_else(|| anyhow::anyhow!("table {id} not found"))
    }

    fn require_field(&self, table_id: i64, field_id: i64) -> Result<FieldMeta> {
        self.field_by_id(table_id, field_id)?
            .ok_or_else(|| anyhow::anyhow!("field {field_id} on table {table_id} not found"))
    }

    /// Distinct, non-blank values of `field` across the given rows.
    fn column_values_for_rows(
        &self,
        table: &TableMeta,
        field: &FieldMeta,
        row_ids: &[i64],
    ) -> Result<Vec<String>> {
        Ok(self
            .column_values_for_rows_all(table, field, row_ids)?
            .into_iter()
            .filter(|v| !v.is_empty())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect())
    }

    /// Values of `field` for each row in `row_ids`, in id order (parallels
    /// `row_ids`; not deduplicated).
    fn column_values_for_rows_all(
        &self,
        table: &TableMeta,
        field: &FieldMeta,
        row_ids: &[i64],
    ) -> Result<Vec<String>> {
        if row_ids.is_empty() {
            return Ok(Vec::new());
        }
        let marks: Vec<String> = (1..=row_ids.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT {} FROM {} WHERE id IN ({}) ORDER BY id",
            field.phys,
            table.phys,
            marks.join(", ")
        );
        let mut stmt = self.data.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(row_ids.iter()), |row| {
            Ok(cell_string(row.get_ref(0)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Rowids of `table` whose `field` matches any of `keys`, in id order.
    fn row_ids_where_value_in(
        &self,
        table: &TableMeta,
        field: &FieldMeta,
        keys: &[String],
    ) -> Result<Vec<i64>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let marks: Vec<String> = (1..=keys.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT id FROM {} WHERE {} IN ({}) ORDER BY id",
            table.phys,
            field.phys,
            marks.join(", ")
        );
        let mut stmt = self.data.prepare(&sql)?;
        let ps: Vec<Value> = keys.iter().map(|k| Value::Text(k.clone())).collect();
        let rows = stmt.query_map(params_from_iter(ps), |r| r.get::<_, i64>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

/// Stringify a cell value (mirrors `data::value_to_string`); Null → empty.
fn cell_string(v: ValueRef<'_>) -> String {
    match v {
        ValueRef::Null => String::new(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
        ValueRef::Blob(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use crate::model::Cardinality;
    use crate::options::FieldOptions;
    use crate::path::{HopDirection, PathError, RouteClass};
    use crate::{FieldKind, NewField, NewRelationship, Solution};

    /// A field on `table_id` by logical name (test helper).
    fn field_id(s: &Solution, table_id: i64, name: &str) -> i64 {
        s.all_fields(table_id)
            .unwrap()
            .into_iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("no field {name}"))
            .id
    }

    /// The system primary-key field id of `table_id`.
    fn system_pk(s: &Solution, table_id: i64) -> i64 {
        s.all_fields(table_id)
            .unwrap()
            .into_iter()
            .find(|f| FieldOptions::parse(&f.options).system)
            .unwrap()
            .id
    }

    /// A table's system-PK UUID for the row at `row_id`.
    fn pk_value(s: &Solution, table_id: i64, row_id: i64) -> String {
        let table = s.table_by_id(table_id).unwrap().unwrap();
        let all = s.all_fields(table_id).unwrap();
        let pk_idx = all
            .iter()
            .position(|f| FieldOptions::parse(&f.options).system)
            .unwrap();
        s.get_record(&table, &all, row_id).unwrap().unwrap()[pk_idx].clone()
    }

    /// Build a two-table Invoice→Customer schema linked by a direct FK. Returns
    /// (invoices_id, customers_id).
    fn invoice_customer(s: &mut Solution) -> (i64, i64) {
        let customers = s
            .create_table("Customers", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let invoices = s
            .create_table(
                "Invoices",
                &[
                    NewField { name: "Total".into(), kind: FieldKind::Number },
                    NewField { name: "CustomerId".into(), kind: FieldKind::Text },
                ],
            )
            .unwrap();
        s.create_relationship(&NewRelationship {
            name: "customer".into(),
            from_table: invoices,
            to_table: customers,
            from_field: field_id(s, invoices, "CustomerId"),
            to_field: system_pk(s, customers),
        })
        .unwrap()
        .unwrap();
        (invoices, customers)
    }

    #[test]
    fn direct_forward_path_resolves_to_one_value() {
        let mut s = Solution::open_in_memory().unwrap();
        let (invoices, customers) = invoice_customer(&mut s);

        let cust_tbl = s.table_by_id(customers).unwrap().unwrap();
        let cust_name = s.field_by_id(customers, field_id(&s, customers, "Name")).unwrap().unwrap();
        let ada = s.insert_record(&cust_tbl, &[(&cust_name, "Ada".into())]).unwrap();
        let ada_uuid = pk_value(&s, customers, ada);

        let inv_tbl = s.table_by_id(invoices).unwrap().unwrap();
        let inv_fk = s.field_by_id(invoices, field_id(&s, invoices, "CustomerId")).unwrap().unwrap();
        let inv = s.insert_record(&inv_tbl, &[(&inv_fk, ada_uuid.clone())]).unwrap();

        // Leading base-table name is optional and case-insensitive.
        let route = s.resolve_path(invoices, "invoices.customer.name").unwrap();
        assert_eq!(route.hops.len(), 1);
        assert_eq!(route.hops[0].direction, HopDirection::Forward);
        assert_eq!(route.hops[0].cardinality, Cardinality::ToOne);
        assert_eq!(route.terminal_table, customers);
        assert_eq!(route.class, RouteClass::DirectFk);
        assert!(route.class.create_determined());

        assert_eq!(s.route_record_set(&route, inv).unwrap(), vec![ada]);
        assert_eq!(s.route_values(&route, inv).unwrap(), vec!["Ada".to_string()]);

        // Same route with an explicit leading table name resolves identically.
        let route2 = s.resolve_path(invoices, "customer.name").unwrap();
        assert_eq!(route2.class, RouteClass::DirectFk);
        assert_eq!(s.route_values(&route2, inv).unwrap(), vec!["Ada".to_string()]);
    }

    #[test]
    fn direct_reverse_path_resolves_to_many_set() {
        let mut s = Solution::open_in_memory().unwrap();
        let (invoices, customers) = invoice_customer(&mut s);

        let cust_tbl = s.table_by_id(customers).unwrap().unwrap();
        let cust_name = s.field_by_id(customers, field_id(&s, customers, "Name")).unwrap().unwrap();
        let ada = s.insert_record(&cust_tbl, &[(&cust_name, "Ada".into())]).unwrap();
        let ada_uuid = pk_value(&s, customers, ada);

        let inv_tbl = s.table_by_id(invoices).unwrap().unwrap();
        let inv_fk = s.field_by_id(invoices, field_id(&s, invoices, "CustomerId")).unwrap().unwrap();
        let i1 = s.insert_record(&inv_tbl, &[(&inv_fk, ada_uuid.clone())]).unwrap();
        let i2 = s.insert_record(&inv_tbl, &[(&inv_fk, ada_uuid.clone())]).unwrap();
        // An unrelated invoice (different/blank customer) is excluded.
        s.insert_record(&inv_tbl, &[(&inv_fk, "".into())]).unwrap();

        // From the Customer side the same declared relationship is reverse/to-many.
        let route = s.resolve_path(customers, "Customers.customer").unwrap();
        assert_eq!(route.hops.len(), 1);
        assert_eq!(route.hops[0].direction, HopDirection::Reverse);
        assert_eq!(route.hops[0].cardinality, Cardinality::ToMany);
        assert_eq!(route.terminal_table, invoices);
        assert_eq!(route.class, RouteClass::DirectFk);

        assert_eq!(s.route_record_set(&route, ada).unwrap(), vec![i1, i2]);
    }

    /// Student → Enrollments (to-many) → Course (to-one): a join-table M:N.
    fn student_course(s: &mut Solution) -> (i64, i64, i64) {
        let students = s
            .create_table("Students", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let courses = s
            .create_table("Courses", &[NewField { name: "Title".into(), kind: FieldKind::Text }])
            .unwrap();
        let enrollments = s
            .create_table(
                "Enrollments",
                &[
                    NewField { name: "StudentId".into(), kind: FieldKind::Text },
                    NewField { name: "CourseId".into(), kind: FieldKind::Text },
                ],
            )
            .unwrap();
        s.create_relationship(&NewRelationship {
            name: "student".into(),
            from_table: enrollments,
            to_table: students,
            from_field: field_id(s, enrollments, "StudentId"),
            to_field: system_pk(s, students),
        })
        .unwrap()
        .unwrap();
        s.create_relationship(&NewRelationship {
            name: "course".into(),
            from_table: enrollments,
            to_table: courses,
            from_field: field_id(s, enrollments, "CourseId"),
            to_field: system_pk(s, courses),
        })
        .unwrap()
        .unwrap();
        (students, courses, enrollments)
    }

    #[test]
    fn join_table_many_to_many_is_determined_and_resolves() {
        let mut s = Solution::open_in_memory().unwrap();
        let (students, courses, enrollments) = student_course(&mut s);

        let s_tbl = s.table_by_id(students).unwrap().unwrap();
        let s_name = s.field_by_id(students, field_id(&s, students, "Name")).unwrap().unwrap();
        let ada = s.insert_record(&s_tbl, &[(&s_name, "Ada".into())]).unwrap();
        let ada_uuid = pk_value(&s, students, ada);

        let c_tbl = s.table_by_id(courses).unwrap().unwrap();
        let c_title = s.field_by_id(courses, field_id(&s, courses, "Title")).unwrap().unwrap();
        let math = s.insert_record(&c_tbl, &[(&c_title, "Math".into())]).unwrap();
        let art = s.insert_record(&c_tbl, &[(&c_title, "Art".into())]).unwrap();
        let unrelated = s.insert_record(&c_tbl, &[(&c_title, "Ghost".into())]).unwrap();
        let _ = unrelated;
        let math_uuid = pk_value(&s, courses, math);
        let art_uuid = pk_value(&s, courses, art);

        let e_tbl = s.table_by_id(enrollments).unwrap().unwrap();
        let e_sid = s.field_by_id(enrollments, field_id(&s, enrollments, "StudentId")).unwrap().unwrap();
        let e_cid = s.field_by_id(enrollments, field_id(&s, enrollments, "CourseId")).unwrap().unwrap();
        s.insert_record(&e_tbl, &[(&e_sid, ada_uuid.clone()), (&e_cid, math_uuid.clone())]).unwrap();
        s.insert_record(&e_tbl, &[(&e_sid, ada_uuid.clone()), (&e_cid, art_uuid.clone())]).unwrap();

        let route = s.resolve_path(students, "Students.student.course.title").unwrap();
        assert_eq!(route.hops.len(), 2);
        assert_eq!(route.hops[0].cardinality, Cardinality::ToMany);
        assert_eq!(route.hops[1].cardinality, Cardinality::ToOne);
        assert_eq!(route.terminal_table, courses);
        assert_eq!(route.class, RouteClass::JoinTableManyToMany);
        assert!(route.class.create_determined());

        assert_eq!(s.route_record_set(&route, ada).unwrap(), vec![math, art]);
        let mut vals = s.route_values(&route, ada).unwrap();
        vals.sort();
        assert_eq!(vals, vec!["Art".to_string(), "Math".to_string()]);
    }

    #[test]
    fn to_many_chain_is_not_determined() {
        let mut s = Solution::open_in_memory().unwrap();
        let companies = s
            .create_table("Companies", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let departments = s
            .create_table(
                "Departments",
                &[NewField { name: "CompanyId".into(), kind: FieldKind::Text }],
            )
            .unwrap();
        let employees = s
            .create_table(
                "Employees",
                &[NewField { name: "DeptId".into(), kind: FieldKind::Text }],
            )
            .unwrap();
        s.create_relationship(&NewRelationship {
            name: "company".into(),
            from_table: departments,
            to_table: companies,
            from_field: field_id(&s, departments, "CompanyId"),
            to_field: system_pk(&s, companies),
        })
        .unwrap()
        .unwrap();
        s.create_relationship(&NewRelationship {
            name: "department".into(),
            from_table: employees,
            to_table: departments,
            from_field: field_id(&s, employees, "DeptId"),
            to_field: system_pk(&s, departments),
        })
        .unwrap()
        .unwrap();

        // Company → its departments (to-many) → their employees (to-many).
        let route = s.resolve_path(companies, "Companies.company.department").unwrap();
        assert_eq!(route.hops.len(), 2);
        assert_eq!(route.hops[0].cardinality, Cardinality::ToMany);
        assert_eq!(route.hops[1].cardinality, Cardinality::ToMany);
        assert_eq!(route.terminal_table, employees);
        assert_eq!(route.class, RouteClass::Undetermined);
        assert!(!route.class.create_determined());
    }

    #[test]
    fn base_field_route_has_no_hops() {
        let mut s = Solution::open_in_memory().unwrap();
        let (invoices, _customers) = invoice_customer(&mut s);
        let inv_tbl = s.table_by_id(invoices).unwrap().unwrap();
        let total = s.field_by_id(invoices, field_id(&s, invoices, "Total")).unwrap().unwrap();
        let inv = s.insert_record(&inv_tbl, &[(&total, "42".into())]).unwrap();

        let route = s.resolve_path(invoices, "Invoices.Total").unwrap();
        assert!(route.hops.is_empty());
        assert_eq!(route.terminal_table, invoices);
        assert_eq!(route.class, RouteClass::BaseRecord);
        assert!(!route.class.create_determined());
        assert_eq!(s.route_record_set(&route, inv).unwrap(), vec![inv]);
        assert_eq!(s.route_values(&route, inv).unwrap(), vec!["42".to_string()]);
    }

    #[test]
    fn unknown_and_misplaced_segments_error() {
        let mut s = Solution::open_in_memory().unwrap();
        let (invoices, _customers) = invoice_customer(&mut s);

        let nope = s.resolve_path(invoices, "Invoices.nope").unwrap_err();
        assert_eq!(nope.downcast_ref::<PathError>(), Some(&PathError::UnknownSegment("nope".into())));

        // A field mid-path (Total is a base field, not a relationship) is refused.
        let mid = s.resolve_path(invoices, "Invoices.Total.customer").unwrap_err();
        assert_eq!(
            mid.downcast_ref::<PathError>(),
            Some(&PathError::FieldNotTerminal("Total".into()))
        );

        let empty = s.resolve_path(invoices, "").unwrap_err();
        assert_eq!(empty.downcast_ref::<PathError>(), Some(&PathError::Empty));
    }
}
