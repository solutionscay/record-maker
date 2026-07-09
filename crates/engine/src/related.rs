//! #111 — the related-record CRUD engine (the heart of the #108 epic).
//!
//! Read / create / update / delete related records through an **anchor** — a
//! resolved route through the FK graph ([`ResolvedRoute`], #11) rooted at one
//! base record. The engine enforces the *determined-stamp* create law and the
//! relationship's referential flags (`allow_create` / `allow_delete`, #110), so
//! the portal object and any programmatic caller get identical semantics.
//!
//! Relationships anchor on the system UUID primary key (#155): every hop matches
//! an FK column against a referenced key column, never a physical rowid.
//!
//! ## What each verb acts on
//!
//! * **Read** — the terminal record set of the route (direct or join-table),
//!   each row addressable for edit through the normal child-record lifecycle.
//! * **Create** — allowed *iff* the route is create-determined (#11): a direct
//!   FK or a join-table M:N. Two paths, both supported where determined:
//!   * **create-new** mints a terminal record, and
//!   * **associate-existing** links an already-existing terminal (e.g. picked
//!     from a value list) — no new terminal minted.
//!   A to-many chain (undetermined parent) is refused; so is a base-record route
//!   with no hops.
//! * **Update** — open/commit a terminal row's fields, scoped to its table.
//! * **Delete** — the *nearest* record: a join-table M:N unlinks the join row
//!   (never cascading to the terminal); a direct to-many deletes the child row;
//!   a direct to-one unlinks the base's FK. Never cascades outward.

use anyhow::{bail, Result};
use rusqlite::params_from_iter;
use rusqlite::types::Value;

use crate::model::{FieldMeta, RelationshipMeta, TableMeta};
use crate::options::ValidationMode;
use crate::path::{HopDirection, ResolvedRoute, RouteClass};
use crate::Record;
use crate::Solution;

/// Why a related-record CRUD operation was refused.
///
/// Downcastable off the returned `anyhow::Error` so callers (the portal object,
/// the server routes) can map refusals to a precise response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelatedCrudError {
    /// The route addresses the base record itself (no relationship hops); there
    /// is no *related* record to create or delete through it.
    NotARelatedRoute,
    /// Create was requested on a route whose immediate parent is undetermined (a
    /// multi-hop route crossing a to-many boundary). The stamp cannot be
    /// inverted, so create is refused.
    CreateUndetermined,
    /// `allow_create` is off on the anchoring relationship.
    CreateNotAllowed,
    /// `allow_delete` is off on the anchoring relationship.
    DeleteNotAllowed,
}

impl std::fmt::Display for RelatedCrudError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotARelatedRoute => {
                f.write_str("route has no relationship hops (base-record route)")
            }
            Self::CreateUndetermined => {
                f.write_str("create refused: route parent is undetermined (multi-hop to-many)")
            }
            Self::CreateNotAllowed => f.write_str("create refused: allow_create is off"),
            Self::DeleteNotAllowed => f.write_str("delete refused: allow_delete is off"),
        }
    }
}

impl std::error::Error for RelatedCrudError {}

/// A **display-only** refinement (#112) layered on a related-data route.
///
/// A read-time predicate that narrows *which* related records are shown, and
/// nothing else. It is a conjunction (AND) of [`FilterClause`]s evaluated over
/// the terminal record set produced by the route (#11). An empty filter passes
/// every record through.
///
/// By construction the filter touches only [`Solution::read_related_records_filtered`];
/// create / update / delete never consult it, so a row created through the
/// anchor that the filter would exclude is still a real, editable record via its
/// route — it simply does not display. That is honest and expected: membership
/// is defined solely by the route's FK equality; the filter is a lens over the
/// read.
///
/// It is persisted per-use on the portal/source object props, never on the
/// catalogued relationship — hence it is passed in at read time rather than read
/// off the [`ResolvedRoute`].
#[derive(Debug, Clone, Default)]
pub struct RelatedFilter {
    /// Conjunctive clauses; all must hold for a record to display. Empty ⇒ no
    /// refinement (every record passes).
    pub clauses: Vec<FilterClause>,
}

impl RelatedFilter {
    /// A filter that narrows nothing (passes every record).
    pub fn none() -> Self {
        Self::default()
    }

    /// Whether this filter refines anything at all.
    pub fn is_empty(&self) -> bool {
        self.clauses.is_empty()
    }
}

/// One predicate in a [`RelatedFilter`]: a user field on the route's terminal
/// table, an operator, and the right-hand operand it is compared against.
#[derive(Debug, Clone)]
pub struct FilterClause {
    /// A user field on the route's *terminal* table whose value is tested.
    pub field_id: i64,
    /// The comparison operator.
    pub op: FilterOp,
    /// What the terminal field's value is compared against.
    pub rhs: FilterOperand,
}

/// The comparison operators a filter clause may use. Deliberately narrow —
/// equality plus the ordered comparisons; a full expression language is future
/// work (#108).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// The right-hand side of a [`FilterClause`].
#[derive(Debug, Clone)]
pub enum FilterOperand {
    /// A literal comparison value.
    Value(String),
    /// A field on the *base* (parent) record, read once per base record; lets a
    /// portal filter its related rows against a value on the record it hangs
    /// off (e.g. show only children whose `status` equals the parent's `stage`).
    ParentField(i64),
}

/// Compare `lhs` against `rhs` under `op`. Both operands parse to `f64` ⇒ an
/// ordered numeric comparison (so numeric/bool ranges work); otherwise a
/// byte-wise string comparison. A non-orderable result (NaN) never matches.
fn filter_matches(lhs: &str, op: FilterOp, rhs: &str) -> bool {
    use std::cmp::Ordering::*;
    let ord = match (lhs.parse::<f64>(), rhs.parse::<f64>()) {
        (Ok(a), Ok(b)) => a.partial_cmp(&b),
        _ => Some(lhs.cmp(rhs)),
    };
    let Some(ord) = ord else { return false };
    match op {
        FilterOp::Eq => ord == Equal,
        FilterOp::Ne => ord != Equal,
        FilterOp::Lt => ord == Less,
        FilterOp::Le => ord != Greater,
        FilterOp::Gt => ord == Greater,
        FilterOp::Ge => ord != Less,
    }
}

impl Solution {
    /// **Read** the related record set for `route` from base record `base_id`,
    /// as full [`Record`]s of the terminal table (user fields, in field order),
    /// each addressable for edit. Membership is exactly the route's resolved set
    /// (#11); ordering follows the terminal table's id order.
    pub fn read_related_records(&self, route: &ResolvedRoute, base_id: i64) -> Result<Vec<Record>> {
        self.read_related_records_filtered(route, base_id, &RelatedFilter::none())
    }

    /// **Read** the related record set for `route` from base record `base_id`,
    /// narrowed by a **display-only** `filter` (#112).
    ///
    /// The route's FK-equality set (#11) defines membership; `filter` is a
    /// conjunctive, read-time lens that hides records not satisfying every
    /// clause. It **never** participates in create / update / delete — those
    /// verbs do not call this method and are structurally unable to consult the
    /// filter. An empty filter is exactly [`read_related_records`].
    ///
    /// Each clause names a user field on the terminal table; a
    /// [`FilterOperand::ParentField`] compares against a field on the base
    /// record, read once. An unknown terminal-field id is an error (a malformed
    /// filter), not a silent all-exclude.
    pub fn read_related_records_filtered(
        &self,
        route: &ResolvedRoute,
        base_id: i64,
        filter: &RelatedFilter,
    ) -> Result<Vec<Record>> {
        let ids = self.route_record_set(route, base_id)?;
        let table = self.related_table(route.terminal_table)?;
        let fields = self.fields(table.id)?;
        let plan = self.compile_filter(route, base_id, &fields, filter)?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(cells) = self.get_record(&table, &fields, id)? {
                if plan.iter().all(|(idx, op, rhs)| {
                    cells
                        .get(*idx)
                        .is_some_and(|lhs| filter_matches(lhs, *op, rhs))
                }) {
                    out.push(Record { id, cells });
                }
            }
        }
        Ok(out)
    }

    /// Resolve a [`RelatedFilter`] against the terminal `fields` (in cell order)
    /// and the base record, into `(cell_index, op, rhs_value)` tuples ready to
    /// test each terminal row. Parent-field operands are read once here.
    fn compile_filter(
        &self,
        route: &ResolvedRoute,
        base_id: i64,
        fields: &[FieldMeta],
        filter: &RelatedFilter,
    ) -> Result<Vec<(usize, FilterOp, String)>> {
        let mut plan = Vec::with_capacity(filter.clauses.len());
        for clause in &filter.clauses {
            let idx = fields
                .iter()
                .position(|f| f.id == clause.field_id)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "filter field {} is not a user field on the terminal table",
                        clause.field_id
                    )
                })?;
            let rhs = match &clause.rhs {
                FilterOperand::Value(v) => v.clone(),
                FilterOperand::ParentField(field_id) => {
                    self.base_field_value(route.base_table, *field_id, base_id)?
                }
            };
            plan.push((idx, clause.op, rhs));
        }
        Ok(plan)
    }

    /// The value of a field on the base (parent) record; empty string when the
    /// row or value is absent. Used to resolve [`FilterOperand::ParentField`].
    fn base_field_value(&self, table_id: i64, field_id: i64, row_id: i64) -> Result<String> {
        let table = self.related_table(table_id)?;
        let field = self.related_field(table_id, field_id)?;
        Ok(self
            .get_record(&table, std::slice::from_ref(&field), row_id)?
            .and_then(|cells| cells.into_iter().next())
            .unwrap_or_default())
    }

    /// **Create** a related record through `route` from base record `base_id`.
    ///
    /// Create is offered only when the route is create-determined (#11) — a
    /// direct FK or a join-table M:N — and the anchoring relationship's
    /// `allow_create` is on. Two paths:
    ///
    /// * `associate_existing = Some(terminal_id)` links the already-existing
    ///   terminal record (set the join FK / base FK); `values` is ignored.
    /// * `associate_existing = None` mints a new terminal record from `values`
    ///   and links it.
    ///
    /// The written FK(s) are stamped from determined keys so the new row belongs.
    /// A join-table M:N mints the terminal and the join row **atomically** in one
    /// transaction. Returns the id of the terminal record that now belongs (the
    /// newly minted or the associated one).
    ///
    /// Errors with a downcastable [`RelatedCrudError`] when the route is not
    /// create-determined or `allow_create` is off.
    pub fn create_related_record(
        &self,
        route: &ResolvedRoute,
        base_id: i64,
        values: &[(&FieldMeta, String)],
        associate_existing: Option<i64>,
    ) -> Result<i64> {
        self.create_related_record_mode(route, base_id, values, associate_existing, ValidationMode::Full)
    }

    /// Mint a related record as a DRAFT (#173): identical to
    /// [`Solution::create_related_record`] — same determined-stamp gate, FK
    /// stamping and (for M:N) atomic join row — but the minted terminal (and any
    /// rows written on its behalf) defer required + uniqueness to the record-EXIT
    /// commit, so a portal's trailing blank row can create a related record even
    /// when the terminal table has a required field. Returns the terminal id to
    /// register in the caller's draft set.
    pub fn create_related_record_draft(
        &self,
        route: &ResolvedRoute,
        base_id: i64,
        values: &[(&FieldMeta, String)],
        associate_existing: Option<i64>,
    ) -> Result<i64> {
        self.create_related_record_mode(route, base_id, values, associate_existing, ValidationMode::Draft)
    }

    fn create_related_record_mode(
        &self,
        route: &ResolvedRoute,
        base_id: i64,
        values: &[(&FieldMeta, String)],
        associate_existing: Option<i64>,
        mode: ValidationMode,
    ) -> Result<i64> {
        if route.hops.is_empty() {
            bail!(RelatedCrudError::NotARelatedRoute);
        }
        if !route.class.create_determined() {
            bail!(RelatedCrudError::CreateUndetermined);
        }

        match route.class {
            RouteClass::DirectFk => {
                let hop = &route.hops[0];
                let rel = self.require_relationship(hop.relationship_id)?;
                if !rel.allow_create {
                    bail!(RelatedCrudError::CreateNotAllowed);
                }
                match hop.direction {
                    // base is the parent; the child (from_table) carries the FK.
                    // Stamp the child's FK with the base record's key.
                    HopDirection::Reverse => {
                        let base_key =
                            self.key_value(rel.to_table, rel.to_field, base_id)?;
                        let child_table = self.related_table(rel.from_table)?;
                        let fk = self.related_field(rel.from_table, rel.from_field)?;
                        match associate_existing {
                            Some(existing) => {
                                self.update_record(
                                    &child_table,
                                    existing,
                                    &[(&fk, base_key)],
                                )?;
                                Ok(existing)
                            }
                            None => {
                                self.insert_stamped_mode(&child_table, values, &fk, base_key, mode)
                            }
                        }
                    }
                    // base is the child; the parent (to_table) is the related
                    // record. Point the base's FK at the (new or existing) parent.
                    HopDirection::Forward => {
                        let parent_table = self.related_table(rel.to_table)?;
                        let base_table = self.related_table(rel.from_table)?;
                        let base_fk = self.related_field(rel.from_table, rel.from_field)?;
                        let parent_id = match associate_existing {
                            Some(existing) => existing,
                            None => self.insert_record_mode(&parent_table, values, mode)?,
                        };
                        let parent_key =
                            self.key_value(rel.to_table, rel.to_field, parent_id)?;
                        self.update_record_mode(
                            &base_table,
                            base_id,
                            &[(&base_fk, parent_key)],
                            mode,
                        )?;
                        Ok(parent_id)
                    }
                }
            }
            // A → to-many join → to-one terminal. Mint the terminal (or take the
            // associated one) and mint the join row stamping both determined FKs,
            // atomically.
            RouteClass::JoinTableManyToMany => {
                let anchor = self.require_relationship(route.hops[0].relationship_id)?;
                let terminal = self.require_relationship(route.hops[1].relationship_id)?;
                if !anchor.allow_create {
                    bail!(RelatedCrudError::CreateNotAllowed);
                }
                let base_key = self.key_value(anchor.to_table, anchor.to_field, base_id)?;
                let join_table = self.related_table(anchor.from_table)?;
                let join_fk_base = self.related_field(anchor.from_table, anchor.from_field)?;
                let join_fk_term = self.related_field(terminal.from_table, terminal.from_field)?;
                let terminal_table = self.related_table(terminal.to_table)?;

                self.in_transaction(|s| {
                    let terminal_id = match associate_existing {
                        Some(existing) => existing,
                        None => s.insert_record_mode(&terminal_table, values, mode)?,
                    };
                    let terminal_key =
                        s.key_value(terminal.to_table, terminal.to_field, terminal_id)?;
                    s.insert_record_mode(
                        &join_table,
                        &[(&join_fk_base, base_key), (&join_fk_term, terminal_key)],
                        mode,
                    )?;
                    Ok(terminal_id)
                })
            }
            RouteClass::BaseRecord | RouteClass::Undetermined => {
                // Already handled above, but keep the match exhaustive.
                bail!(RelatedCrudError::CreateUndetermined)
            }
        }
    }

    /// **Update** a related row's fields, scoped to the route's terminal table,
    /// through the normal child-record lifecycle (validation, system-PK
    /// protection). `record_id` is a terminal-table row from [`read_related_records`].
    pub fn update_related_record(
        &self,
        route: &ResolvedRoute,
        record_id: i64,
        values: &[(&FieldMeta, String)],
    ) -> Result<()> {
        let table = self.related_table(route.terminal_table)?;
        self.update_record(&table, record_id, values)
    }

    /// **Delete** the nearest record for `route` / base `base_id`:
    ///
    /// * join-table M:N — unlink the join row between the base and `record_id`
    ///   (the terminal is untouched — no cascade);
    /// * direct to-many — delete the child row `record_id`;
    /// * direct to-one — unlink the base's FK (the parent is untouched).
    ///
    /// Gated by the anchoring relationship's `allow_delete`. Errors with a
    /// downcastable [`RelatedCrudError`] when the route has no hops or delete is
    /// not allowed.
    pub fn delete_related_record(
        &self,
        route: &ResolvedRoute,
        base_id: i64,
        record_id: i64,
    ) -> Result<()> {
        if route.hops.is_empty() {
            bail!(RelatedCrudError::NotARelatedRoute);
        }
        let anchor = self.require_relationship(route.hops[0].relationship_id)?;
        if !anchor.allow_delete {
            bail!(RelatedCrudError::DeleteNotAllowed);
        }

        match route.class {
            RouteClass::DirectFk => {
                let hop = &route.hops[0];
                match hop.direction {
                    // The child row is the nearest (and only) record — delete it.
                    HopDirection::Reverse => {
                        let child_table = self.related_table(anchor.from_table)?;
                        self.delete_record(&child_table, record_id)
                    }
                    // The parent may be shared — unlink by clearing the base FK.
                    HopDirection::Forward => {
                        let base_table = self.related_table(anchor.from_table)?;
                        let base_fk = self.related_field(anchor.from_table, anchor.from_field)?;
                        self.update_record(&base_table, base_id, &[(&base_fk, String::new())])
                    }
                }
            }
            // Nearest record is the join row: delete only the row linking base and
            // this terminal. Never cascade to the terminal record.
            RouteClass::JoinTableManyToMany => {
                let terminal = self.require_relationship(route.hops[1].relationship_id)?;
                let base_key = self.key_value(anchor.to_table, anchor.to_field, base_id)?;
                let terminal_key =
                    self.key_value(terminal.to_table, terminal.to_field, record_id)?;
                let join_table = self.related_table(anchor.from_table)?;
                let join_fk_base = self.related_field(anchor.from_table, anchor.from_field)?;
                let join_fk_term = self.related_field(terminal.from_table, terminal.from_field)?;
                let sql = format!(
                    "DELETE FROM {} WHERE {}=?1 AND {}=?2",
                    join_table.phys, join_fk_base.phys, join_fk_term.phys
                );
                self.data.execute(
                    &sql,
                    params_from_iter([Value::Text(base_key), Value::Text(terminal_key)]),
                )?;
                Ok(())
            }
            RouteClass::BaseRecord | RouteClass::Undetermined => {
                // BaseRecord has no hops (handled above). An undetermined route
                // still resolves to a real terminal record set for delete, but the
                // nearest-record rule needs a determined anchor to be unambiguous;
                // treat it as a plain delete of the terminal is out of scope here.
                bail!(RelatedCrudError::NotARelatedRoute)
            }
        }
    }

    // --- helpers -----------------------------------------------------------

    /// Insert into `table` with `values`, overriding/stamping `fk` = `key`,
    /// running the given [`ValidationMode`] (Draft defers required + uniqueness
    /// for the #173 draft mint; the FK stamp itself is unchanged).
    fn insert_stamped_mode(
        &self,
        table: &TableMeta,
        values: &[(&FieldMeta, String)],
        fk: &FieldMeta,
        key: String,
        mode: ValidationMode,
    ) -> Result<i64> {
        let mut vals: Vec<(&FieldMeta, String)> = values
            .iter()
            .filter(|(f, _)| f.id != fk.id)
            .map(|(f, v)| (*f, v.clone()))
            .collect();
        vals.push((fk, key));
        self.insert_record_mode(table, &vals, mode)
    }

    /// The current value of key field `field_id` on row `row_id` of `table_id`
    /// (the referenced side of an FK, i.e. the system PK — #155). Empty string
    /// when the row or value is absent.
    fn key_value(&self, table_id: i64, field_id: i64, row_id: i64) -> Result<String> {
        let table = self.related_table(table_id)?;
        let field = self.related_field(table_id, field_id)?;
        Ok(self
            .get_record(&table, std::slice::from_ref(&field), row_id)?
            .and_then(|cells| cells.into_iter().next())
            .unwrap_or_default())
    }

    fn related_table(&self, table_id: i64) -> Result<TableMeta> {
        self.table_by_id(table_id)?
            .ok_or_else(|| anyhow::anyhow!("table {table_id} not found"))
    }

    fn related_field(&self, table_id: i64, field_id: i64) -> Result<FieldMeta> {
        self.field_by_id(table_id, field_id)?
            .ok_or_else(|| anyhow::anyhow!("field {field_id} on table {table_id} not found"))
    }

    fn require_relationship(&self, id: i64) -> Result<RelationshipMeta> {
        self.relationship_by_id(id)?
            .ok_or_else(|| anyhow::anyhow!("relationship {id} not found"))
    }

    /// Run `f` inside a data.db transaction, committing on `Ok` and rolling back
    /// on `Err`. Uses raw `BEGIN`/`COMMIT`/`ROLLBACK` so the enclosed `&self`
    /// CRUD helpers compose without needing a `&mut` borrow of the connection.
    fn in_transaction<T>(&self, f: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        self.data.execute_batch("BEGIN")?;
        match f(self) {
            Ok(v) => {
                self.data.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.data.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::options::FieldOptions;
    use crate::path::RouteClass;
    use crate::related::{
        FilterClause, FilterOp, FilterOperand, RelatedCrudError, RelatedFilter,
    };
    use crate::{FieldKind, NewField, NewRelationship, Solution};

    fn field_id(s: &Solution, table_id: i64, name: &str) -> i64 {
        s.all_fields(table_id)
            .unwrap()
            .into_iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("no field {name}"))
            .id
    }

    fn field(s: &Solution, table_id: i64, name: &str) -> crate::FieldMeta {
        s.field_by_id(table_id, field_id(s, table_id, name))
            .unwrap()
            .unwrap()
    }

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

    /// Read the raw FK value stored on one row of a table.
    fn fk_of(s: &Solution, table_id: i64, fk_name: &str, row_id: i64) -> String {
        let table = s.table_by_id(table_id).unwrap().unwrap();
        let f = field(s, table_id, fk_name);
        s.get_record(&table, std::slice::from_ref(&f), row_id)
            .unwrap()
            .unwrap()[0]
            .clone()
    }

    /// Customers ← Invoices via a direct FK. Returns (invoices, customers, rel_id).
    fn invoice_customer(s: &mut Solution) -> (i64, i64, i64) {
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
        let rel = s
            .create_relationship(&NewRelationship {
                name: "customer".into(),
                from_table: invoices,
                to_table: customers,
                from_field: field_id(s, invoices, "CustomerId"),
                to_field: system_pk(s, customers),
            })
            .unwrap()
            .unwrap()
            .id;
        (invoices, customers, rel)
    }

    /// Insert one customer, return (row_id, uuid).
    fn a_customer(s: &Solution, customers: i64, name: &str) -> (i64, String) {
        let tbl = s.table_by_id(customers).unwrap().unwrap();
        let f = field(s, customers, "Name");
        let id = s.insert_record(&tbl, &[(&f, name.into())]).unwrap();
        (id, pk_value(s, customers, id))
    }

    #[test]
    fn direct_reverse_create_stamps_child_fk() {
        let mut s = Solution::open_in_memory().unwrap();
        let (invoices, customers, rel) = invoice_customer(&mut s);
        s.set_relationship_referential(rel, true, true).unwrap();
        let (ada, ada_uuid) = a_customer(&s, customers, "Ada");

        // Anchor: Customer → its invoices (reverse to-many).
        let route = s.resolve_path(customers, "customer").unwrap();
        assert_eq!(route.class, RouteClass::DirectFk);

        let total = field(&s, invoices, "Total");
        let new_id = s
            .create_related_record(&route, ada, &[(&total, "42".into())], None)
            .unwrap();

        // The new invoice carries Ada's key and shows up in the read set.
        assert_eq!(fk_of(&s, invoices, "CustomerId", new_id), ada_uuid);
        let read = s.read_related_records(&route, ada).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].id, new_id);
    }

    #[test]
    fn create_refused_when_allow_create_off() {
        let mut s = Solution::open_in_memory().unwrap();
        let (invoices, customers, _rel) = invoice_customer(&mut s);
        let (ada, _) = a_customer(&s, customers, "Ada");
        let route = s.resolve_path(customers, "customer").unwrap();
        let total = field(&s, invoices, "Total");

        let err = s
            .create_related_record(&route, ada, &[(&total, "42".into())], None)
            .unwrap_err();
        assert_eq!(
            err.downcast_ref::<RelatedCrudError>(),
            Some(&RelatedCrudError::CreateNotAllowed)
        );
    }

    #[test]
    fn associate_existing_direct_reverse_sets_fk() {
        let mut s = Solution::open_in_memory().unwrap();
        let (invoices, customers, rel) = invoice_customer(&mut s);
        s.set_relationship_referential(rel, true, true).unwrap();
        let (ada, ada_uuid) = a_customer(&s, customers, "Ada");

        // An orphan invoice with no customer.
        let inv_tbl = s.table_by_id(invoices).unwrap().unwrap();
        let total = field(&s, invoices, "Total");
        let orphan = s.insert_record(&inv_tbl, &[(&total, "7".into())]).unwrap();

        let route = s.resolve_path(customers, "customer").unwrap();
        let returned = s
            .create_related_record(&route, ada, &[], Some(orphan))
            .unwrap();
        assert_eq!(returned, orphan);
        assert_eq!(fk_of(&s, invoices, "CustomerId", orphan), ada_uuid);
        assert_eq!(s.read_related_records(&route, ada).unwrap().len(), 1);
    }

    #[test]
    fn direct_reverse_delete_removes_child_and_is_gated() {
        let mut s = Solution::open_in_memory().unwrap();
        let (invoices, customers, rel) = invoice_customer(&mut s);
        // create on, delete off initially.
        s.set_relationship_referential(rel, true, false).unwrap();
        let (ada, _) = a_customer(&s, customers, "Ada");
        let route = s.resolve_path(customers, "customer").unwrap();
        let total = field(&s, invoices, "Total");
        let inv = s
            .create_related_record(&route, ada, &[(&total, "42".into())], None)
            .unwrap();

        // allow_delete off → refused.
        let err = s.delete_related_record(&route, ada, inv).unwrap_err();
        assert_eq!(
            err.downcast_ref::<RelatedCrudError>(),
            Some(&RelatedCrudError::DeleteNotAllowed)
        );

        // Turn it on → the child row is deleted (route must be re-resolved to see
        // the flag; flags live on the relationship meta read at resolve time is
        // not needed — delete reads the live relationship).
        s.set_relationship_referential(rel, true, true).unwrap();
        s.delete_related_record(&route, ada, inv).unwrap();
        assert!(s.read_related_records(&route, ada).unwrap().is_empty());
        let inv_tbl = s.table_by_id(invoices).unwrap().unwrap();
        assert!(s.get_record(&inv_tbl, &[], inv).unwrap().is_none());
    }

    /// Student ← Enrollments → Course: a join-table M:N. Returns
    /// (students, courses, enrollments, rel_student, rel_course).
    fn student_course(s: &mut Solution) -> (i64, i64, i64, i64, i64) {
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
        let rel_student = s
            .create_relationship(&NewRelationship {
                name: "student".into(),
                from_table: enrollments,
                to_table: students,
                from_field: field_id(s, enrollments, "StudentId"),
                to_field: system_pk(s, students),
            })
            .unwrap()
            .unwrap()
            .id;
        let rel_course = s
            .create_relationship(&NewRelationship {
                name: "course".into(),
                from_table: enrollments,
                to_table: courses,
                from_field: field_id(s, enrollments, "CourseId"),
                to_field: system_pk(s, courses),
            })
            .unwrap()
            .unwrap()
            .id;
        (students, courses, enrollments, rel_student, rel_course)
    }

    #[test]
    fn join_mn_create_mints_terminal_and_join() {
        let mut s = Solution::open_in_memory().unwrap();
        let (students, courses, enrollments, rel_student, _rel_course) = student_course(&mut s);
        // Anchor is the first hop (student): enable create+delete on it.
        s.set_relationship_referential(rel_student, true, true).unwrap();

        let s_tbl = s.table_by_id(students).unwrap().unwrap();
        let s_name = field(&s, students, "Name");
        let ada = s.insert_record(&s_tbl, &[(&s_name, "Ada".into())]).unwrap();
        let ada_uuid = pk_value(&s, students, ada);

        let route = s.resolve_path(students, "student.course").unwrap();
        assert_eq!(route.class, RouteClass::JoinTableManyToMany);

        // create-new: mints a Course AND a join row atomically.
        let title = field(&s, courses, "Title");
        let math = s
            .create_related_record(&route, ada, &[(&title, "Math".into())], None)
            .unwrap();

        // The course exists in the read set.
        let read = s.read_related_records(&route, ada).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].id, math);
        assert_eq!(read[0].cells[0], "Math");

        // Exactly one enrollment row wiring Ada ↔ Math.
        let e_tbl = s.table_by_id(enrollments).unwrap().unwrap();
        let n_enroll: i64 = s
            .data
            .query_row(&format!("SELECT COUNT(*) FROM {}", e_tbl.phys), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n_enroll, 1);
        let math_uuid = pk_value(&s, courses, math);
        let sid = fk_of(&s, enrollments, "StudentId", 1);
        let cid = fk_of(&s, enrollments, "CourseId", 1);
        assert_eq!(sid, ada_uuid);
        assert_eq!(cid, math_uuid);
    }

    #[test]
    fn join_mn_associate_existing_only_mints_join() {
        let mut s = Solution::open_in_memory().unwrap();
        let (students, courses, enrollments, rel_student, _rc) = student_course(&mut s);
        s.set_relationship_referential(rel_student, true, true).unwrap();

        let s_tbl = s.table_by_id(students).unwrap().unwrap();
        let s_name = field(&s, students, "Name");
        let ada = s.insert_record(&s_tbl, &[(&s_name, "Ada".into())]).unwrap();

        // A pre-existing Course to associate.
        let c_tbl = s.table_by_id(courses).unwrap().unwrap();
        let title = field(&s, courses, "Title");
        let art = s.insert_record(&c_tbl, &[(&title, "Art".into())]).unwrap();

        let route = s.resolve_path(students, "student.course").unwrap();
        let returned = s
            .create_related_record(&route, ada, &[], Some(art))
            .unwrap();
        assert_eq!(returned, art);

        // No new Course minted (still just Art); one join row.
        let n_courses: i64 = s
            .data
            .query_row(&format!("SELECT COUNT(*) FROM {}", c_tbl.phys), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n_courses, 1);
        let e_tbl = s.table_by_id(enrollments).unwrap().unwrap();
        let n_enroll: i64 = s
            .data
            .query_row(&format!("SELECT COUNT(*) FROM {}", e_tbl.phys), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n_enroll, 1);
        assert_eq!(s.read_related_records(&route, ada).unwrap().len(), 1);
    }

    #[test]
    fn join_mn_delete_unlinks_join_not_terminal() {
        let mut s = Solution::open_in_memory().unwrap();
        let (students, courses, enrollments, rel_student, _rc) = student_course(&mut s);
        s.set_relationship_referential(rel_student, true, true).unwrap();

        let s_tbl = s.table_by_id(students).unwrap().unwrap();
        let s_name = field(&s, students, "Name");
        let ada = s.insert_record(&s_tbl, &[(&s_name, "Ada".into())]).unwrap();

        let route = s.resolve_path(students, "student.course").unwrap();
        let title = field(&s, courses, "Title");
        let math = s
            .create_related_record(&route, ada, &[(&title, "Math".into())], None)
            .unwrap();

        s.delete_related_record(&route, ada, math).unwrap();

        // Join row gone, but the Course terminal survives (no cascade).
        let e_tbl = s.table_by_id(enrollments).unwrap().unwrap();
        let n_enroll: i64 = s
            .data
            .query_row(&format!("SELECT COUNT(*) FROM {}", e_tbl.phys), [], |r| r.get(0))
            .unwrap();
        assert_eq!(n_enroll, 0);
        let c_tbl = s.table_by_id(courses).unwrap().unwrap();
        assert!(s.get_record(&c_tbl, &[], math).unwrap().is_some());
        assert!(s.read_related_records(&route, ada).unwrap().is_empty());
    }

    #[test]
    fn to_many_chain_create_is_refused() {
        let mut s = Solution::open_in_memory().unwrap();
        let companies = s
            .create_table("Companies", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let departments = s
            .create_table("Departments", &[NewField { name: "CompanyId".into(), kind: FieldKind::Text }])
            .unwrap();
        let _employees = s
            .create_table("Employees", &[NewField { name: "DeptId".into(), kind: FieldKind::Text }])
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
            from_table: _employees,
            to_table: departments,
            from_field: field_id(&s, _employees, "DeptId"),
            to_field: system_pk(&s, departments),
        })
        .unwrap()
        .unwrap();

        let c_tbl = s.table_by_id(companies).unwrap().unwrap();
        let c_name = field(&s, companies, "Name");
        let acme = s.insert_record(&c_tbl, &[(&c_name, "Acme".into())]).unwrap();

        // Company → departments (to-many) → employees (to-many): undetermined.
        let route = s.resolve_path(companies, "company.department").unwrap();
        assert_eq!(route.class, RouteClass::Undetermined);
        let err = s
            .create_related_record(&route, acme, &[], None)
            .unwrap_err();
        assert_eq!(
            err.downcast_ref::<RelatedCrudError>(),
            Some(&RelatedCrudError::CreateUndetermined)
        );
    }

    #[test]
    fn base_record_route_is_not_related() {
        let mut s = Solution::open_in_memory().unwrap();
        let (invoices, _customers, _rel) = invoice_customer(&mut s);
        let inv_tbl = s.table_by_id(invoices).unwrap().unwrap();
        let total = field(&s, invoices, "Total");
        let inv = s.insert_record(&inv_tbl, &[(&total, "1".into())]).unwrap();

        let route = s.resolve_path(invoices, "Invoices.Total").unwrap();
        assert_eq!(route.class, RouteClass::BaseRecord);
        let err = s.create_related_record(&route, inv, &[], None).unwrap_err();
        assert_eq!(
            err.downcast_ref::<RelatedCrudError>(),
            Some(&RelatedCrudError::NotARelatedRoute)
        );
        let err = s.delete_related_record(&route, inv, inv).unwrap_err();
        assert_eq!(
            err.downcast_ref::<RelatedCrudError>(),
            Some(&RelatedCrudError::NotARelatedRoute)
        );
    }

    #[test]
    fn update_related_record_edits_terminal() {
        let mut s = Solution::open_in_memory().unwrap();
        let (invoices, customers, rel) = invoice_customer(&mut s);
        s.set_relationship_referential(rel, true, true).unwrap();
        let (ada, _) = a_customer(&s, customers, "Ada");
        let route = s.resolve_path(customers, "customer").unwrap();
        let total = field(&s, invoices, "Total");
        let inv = s
            .create_related_record(&route, ada, &[(&total, "42".into())], None)
            .unwrap();

        s.update_related_record(&route, inv, &[(&total, "99".into())])
            .unwrap();
        let read = s.read_related_records(&route, ada).unwrap();
        assert_eq!(read[0].cells[0].parse::<f64>().unwrap(), 99.0);
    }

    #[test]
    fn direct_forward_create_and_delete_relink_base_fk() {
        let mut s = Solution::open_in_memory().unwrap();
        let (invoices, customers, rel) = invoice_customer(&mut s);
        s.set_relationship_referential(rel, true, true).unwrap();

        // A base invoice with no customer yet.
        let inv_tbl = s.table_by_id(invoices).unwrap().unwrap();
        let total = field(&s, invoices, "Total");
        let inv = s.insert_record(&inv_tbl, &[(&total, "42".into())]).unwrap();

        // Forward anchor: Invoice → its customer (to-one).
        let route = s.resolve_path(invoices, "customer").unwrap();
        assert_eq!(route.class, RouteClass::DirectFk);

        // create-new mints a Customer and points the invoice's FK at it.
        let name = field(&s, customers, "Name");
        let new_cust = s
            .create_related_record(&route, inv, &[(&name, "Ada".into())], None)
            .unwrap();
        let ada_uuid = pk_value(&s, customers, new_cust);
        assert_eq!(fk_of(&s, invoices, "CustomerId", inv), ada_uuid);
        let read = s.read_related_records(&route, inv).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].id, new_cust);

        // delete unlinks (clears the base FK); the parent Customer survives.
        s.delete_related_record(&route, inv, new_cust).unwrap();
        assert_eq!(fk_of(&s, invoices, "CustomerId", inv), "");
        assert!(s.read_related_records(&route, inv).unwrap().is_empty());
        let c_tbl = s.table_by_id(customers).unwrap().unwrap();
        assert!(s.get_record(&c_tbl, &[], new_cust).unwrap().is_some());
    }

    // --- #112: display-only filter ----------------------------------------

    #[test]
    fn filter_narrows_read_but_never_crud() {
        let mut s = Solution::open_in_memory().unwrap();
        let (invoices, customers, rel) = invoice_customer(&mut s);
        s.set_relationship_referential(rel, true, true).unwrap();
        let (ada, _) = a_customer(&s, customers, "Ada");
        let route = s.resolve_path(customers, "customer").unwrap();
        let total = field(&s, invoices, "Total");

        // Two of Ada's invoices; one is below the filter threshold.
        let small = s
            .create_related_record(&route, ada, &[(&total, "42".into())], None)
            .unwrap();
        let big = s
            .create_related_record(&route, ada, &[(&total, "100".into())], None)
            .unwrap();

        // An empty filter passes everything (identical to the plain read).
        let all = s
            .read_related_records_filtered(&route, ada, &RelatedFilter::none())
            .unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(s.read_related_records(&route, ada).unwrap().len(), 2);

        // Total > 50 hides the small invoice from the read set.
        let filter = RelatedFilter {
            clauses: vec![FilterClause {
                field_id: total.id,
                op: FilterOp::Gt,
                rhs: FilterOperand::Value("50".into()),
            }],
        };
        let shown = s
            .read_related_records_filtered(&route, ada, &filter)
            .unwrap();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].id, big);

        // The excluded row is still a real, editable record via its route: the
        // filter changed the READ only. Membership (the unfiltered read) is
        // unchanged, and update still reaches the hidden row.
        assert!(s
            .read_related_records(&route, ada)
            .unwrap()
            .iter()
            .any(|r| r.id == small));
        s.update_related_record(&route, small, &[(&total, "43".into())])
            .unwrap();
        assert_eq!(
            fk_of(&s, invoices, "Total", small).parse::<f64>().unwrap(),
            43.0
        );

        // Deleting the visible row is unaffected by the filter, and deleting the
        // filtered-out row still works (delete never consults the filter).
        s.delete_related_record(&route, ada, small).unwrap();
        assert_eq!(s.read_related_records(&route, ada).unwrap().len(), 1);
    }

    #[test]
    fn filter_compares_against_parent_field() {
        let mut s = Solution::open_in_memory().unwrap();
        let (invoices, customers, rel) = invoice_customer(&mut s);
        s.set_relationship_referential(rel, true, true).unwrap();
        // A per-customer threshold field on the parent (base) table.
        s.add_field(
            customers,
            &NewField { name: "MinTotal".into(), kind: FieldKind::Number },
        )
        .unwrap();
        let (ada, _) = a_customer(&s, customers, "Ada");
        let min = field(&s, customers, "MinTotal");
        s.update_record(
            &s.table_by_id(customers).unwrap().unwrap(),
            ada,
            &[(&min, "50".into())],
        )
        .unwrap();

        let route = s.resolve_path(customers, "customer").unwrap();
        let total = field(&s, invoices, "Total");
        s.create_related_record(&route, ada, &[(&total, "42".into())], None)
            .unwrap();
        let big = s
            .create_related_record(&route, ada, &[(&total, "100".into())], None)
            .unwrap();

        // Show only invoices whose Total >= the parent customer's MinTotal.
        let filter = RelatedFilter {
            clauses: vec![FilterClause {
                field_id: total.id,
                op: FilterOp::Ge,
                rhs: FilterOperand::ParentField(min.id),
            }],
        };
        let shown = s
            .read_related_records_filtered(&route, ada, &filter)
            .unwrap();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].id, big);
    }

    #[test]
    fn filter_conjunction_and_string_equality() {
        let mut s = Solution::open_in_memory().unwrap();
        let (invoices, customers, rel) = invoice_customer(&mut s);
        s.set_relationship_referential(rel, true, true).unwrap();
        s.add_field(
            invoices,
            &NewField { name: "Status".into(), kind: FieldKind::Text },
        )
        .unwrap();
        let (ada, _) = a_customer(&s, customers, "Ada");
        let route = s.resolve_path(customers, "customer").unwrap();
        let total = field(&s, invoices, "Total");
        let status = field(&s, invoices, "Status");

        let want = s
            .create_related_record(
                &route,
                ada,
                &[(&total, "100".into()), (&status, "open".into())],
                None,
            )
            .unwrap();
        // Right status, too small.
        s.create_related_record(
            &route,
            ada,
            &[(&total, "10".into()), (&status, "open".into())],
            None,
        )
        .unwrap();
        // Big enough, wrong status.
        s.create_related_record(
            &route,
            ada,
            &[(&total, "200".into()), (&status, "paid".into())],
            None,
        )
        .unwrap();

        let filter = RelatedFilter {
            clauses: vec![
                FilterClause {
                    field_id: status.id,
                    op: FilterOp::Eq,
                    rhs: FilterOperand::Value("open".into()),
                },
                FilterClause {
                    field_id: total.id,
                    op: FilterOp::Ge,
                    rhs: FilterOperand::Value("50".into()),
                },
            ],
        };
        let shown = s
            .read_related_records_filtered(&route, ada, &filter)
            .unwrap();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].id, want);
    }

    #[test]
    fn filter_unknown_terminal_field_errs() {
        let mut s = Solution::open_in_memory().unwrap();
        let (_invoices, customers, rel) = invoice_customer(&mut s);
        s.set_relationship_referential(rel, true, true).unwrap();
        let (ada, _) = a_customer(&s, customers, "Ada");
        let route = s.resolve_path(customers, "customer").unwrap();

        let filter = RelatedFilter {
            clauses: vec![FilterClause {
                field_id: 999_999,
                op: FilterOp::Eq,
                rhs: FilterOperand::Value("x".into()),
            }],
        };
        assert!(s
            .read_related_records_filtered(&route, ada, &filter)
            .is_err());
    }
}
