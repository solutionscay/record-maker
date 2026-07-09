//! Object CRUD — the `impl Solution` block for `meta_object` reads and writes:
//! stacking-order reads, geometry/z/props/binding/content/read_only writers,
//! create/delete, and identity-preserving restore. Every write is
//! layout-scoped, so a stale or foreign id is a no-op.

use anyhow::Result;
use rusqlite::params;

use super::{NewObject, ObjectKind, ObjectMeta, RestoreObject, RestoreResult};
use crate::Solution;

impl Solution {
    /// Objects on a part, in **stacking order** — back→front by `(z, id)` so
    /// overlapping objects paint deterministically (#25/#43). An unrecognised
    /// stored `kind` falls back to `Text` (rendered, never editable).
    pub fn objects(&self, part_id: i64) -> Result<Vec<ObjectMeta>> {
        let mut stmt = self.app.prepare(
            "SELECT id, part_id, kind, x, y, w, h, z, read_only, binding, content, props, parent_object_id \
             FROM meta_object WHERE part_id=?1 ORDER BY z, id",
        )?;
        let rows = stmt.query_map(params![part_id], |r| {
            let kind_s: String = r.get(2)?;
            Ok(ObjectMeta {
                id: r.get(0)?,
                part_id: r.get(1)?,
                kind: ObjectKind::parse(&kind_s).unwrap_or(ObjectKind::Text),
                x: r.get(3)?,
                y: r.get(4)?,
                w: r.get(5)?,
                h: r.get(6)?,
                z: r.get(7)?,
                read_only: r.get::<_, i64>(8)? != 0,
                binding: r.get(9)?,
                content: r.get(10)?,
                props: r.get(11)?,
                parent_object_id: r.get(12)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// A portal's child objects (its authored columns, #168/#169 Model B),
    /// **scoped** to `layout_id`, in visual COLUMN order (`x, y, z, id`). Matches
    /// `parent_object_id = parent_id`, so a portal enumerates the field/label
    /// objects it owns to project its repeating row template in Browse. Empty for
    /// a parent with no children (or an unknown/foreign id).
    pub fn object_children(&self, layout_id: i64, parent_id: i64) -> Result<Vec<ObjectMeta>> {
        let mut stmt = self.app.prepare(
            "SELECT id, part_id, kind, x, y, w, h, z, read_only, binding, content, props, parent_object_id \
             FROM meta_object \
             WHERE parent_object_id=?1 \
               AND part_id IN (SELECT id FROM meta_part WHERE layout_id=?2) \
             ORDER BY x, y, z, id",
        )?;
        let rows = stmt.query_map(params![parent_id, layout_id], |r| {
            let kind_s: String = r.get(2)?;
            Ok(ObjectMeta {
                id: r.get(0)?,
                part_id: r.get(1)?,
                kind: ObjectKind::parse(&kind_s).unwrap_or(ObjectKind::Text),
                x: r.get(3)?,
                y: r.get(4)?,
                w: r.get(5)?,
                h: r.get(6)?,
                z: r.get(7)?,
                read_only: r.get::<_, i64>(8)? != 0,
                binding: r.get(9)?,
                content: r.get(10)?,
                props: r.get(11)?,
                parent_object_id: r.get(12)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Read one object by id, **scoped** to `layout_id` (the part must belong to
    /// the layout). Returns `None` for an unknown/foreign id. Used after a props
    /// edit to re-derive that object's shape style server-side (#49).
    pub fn object_by_id(&self, layout_id: i64, object_id: i64) -> Result<Option<ObjectMeta>> {
        let mut stmt = self.app.prepare(
            "SELECT id, part_id, kind, x, y, w, h, z, read_only, binding, content, props, parent_object_id \
             FROM meta_object \
             WHERE id=?1 AND part_id IN (SELECT id FROM meta_part WHERE layout_id=?2)",
        )?;
        let mut rows = stmt.query_map(params![object_id, layout_id], |r| {
            let kind_s: String = r.get(2)?;
            Ok(ObjectMeta {
                id: r.get(0)?,
                part_id: r.get(1)?,
                kind: ObjectKind::parse(&kind_s).unwrap_or(ObjectKind::Text),
                x: r.get(3)?,
                y: r.get(4)?,
                w: r.get(5)?,
                h: r.get(6)?,
                z: r.get(7)?,
                read_only: r.get::<_, i64>(8)? != 0,
                binding: r.get(9)?,
                content: r.get(10)?,
                props: r.get(11)?,
                parent_object_id: r.get(12)?,
            })
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Persist an object's part-relative geometry (#15) — the canvas commits a
    /// drag/resize through this. Scoped to `layout_id`: the UPDATE only touches an
    /// object that actually belongs to the layout, so a stale or forged id from
    /// another layout is a silent no-op. Returns the number of rows updated (`0`
    /// ⇒ no such object in that layout). `meta_object` stays the authoritative
    /// source Browse renders from, so a committed drag round-trips on reload.
    pub fn set_object_geometry(
        &self,
        layout_id: i64,
        object_id: i64,
        x: i64,
        y: i64,
        w: i64,
        h: i64,
    ) -> Result<usize> {
        let n = self.app.execute(
            "UPDATE meta_object SET x=?1, y=?2, w=?3, h=?4 \
             WHERE id=?5 AND part_id IN (SELECT id FROM meta_part WHERE layout_id=?6)",
            params![x, y, w, h, object_id, layout_id],
        )?;
        Ok(n)
    }

    /// Move an object to a different band on the SAME layout (cross-band drag,
    /// #46): update its `part_id` and part-relative origin in one write. Both the
    /// object and the target part must belong to `layout_id`, else it's a no-op
    /// returning 0 (mirroring the geometry commands' scoping, so a stale/forged
    /// part id can't graft the object onto a foreign layout). Returns rows updated.
    pub fn set_object_part(
        &self,
        layout_id: i64,
        object_id: i64,
        part_id: i64,
        x: i64,
        y: i64,
    ) -> Result<usize> {
        if !self.part_in_layout(part_id, layout_id)? {
            return Ok(0);
        }
        let n = self.app.execute(
            "UPDATE meta_object SET part_id=?1, x=?2, y=?3 \
             WHERE id=?4 AND part_id IN (SELECT id FROM meta_part WHERE layout_id=?5)",
            params![part_id, x, y, object_id, layout_id],
        )?;
        Ok(n)
    }

    /// Persist several objects' geometry atomically (#46) — a group drag/resize
    /// commits in one transaction so a multi-select transform never half-applies.
    /// Each item is `(object_id, x, y, w, h)`; every UPDATE is layout-scoped like
    /// [`Solution::set_object_geometry`], so foreign/unknown ids are no-ops.
    /// Returns the total number of rows updated.
    pub fn set_objects_geometry(
        &mut self,
        layout_id: i64,
        items: &[(i64, i64, i64, i64, i64)],
    ) -> Result<usize> {
        let tx = self.app.transaction()?;
        let mut updated = 0usize;
        {
            let mut stmt = tx.prepare(
                "UPDATE meta_object SET x=?1, y=?2, w=?3, h=?4 \
                 WHERE id=?5 AND part_id IN (SELECT id FROM meta_part WHERE layout_id=?6)",
            )?;
            for &(id, x, y, w, h) in items {
                updated += stmt.execute(params![x, y, w, h, id, layout_id])?;
            }
        }
        tx.commit()?;
        Ok(updated)
    }

    /// Persist several objects' stacking order (`z`) atomically (#83 Arrange
    /// panel). Align/distribute never touch `z`; only the explicit Bring-to-Front
    /// / Send-to-Back / Bring-Forward / Send-Backward commands rewrite it, so the
    /// canvas POSTs the whole part's re-densified `[(id, z)]` in one call. Each
    /// UPDATE is layout-scoped like [`Solution::set_objects_geometry`], so
    /// foreign/unknown ids are no-ops. Returns the total number of rows updated.
    pub fn set_objects_z(&mut self, layout_id: i64, items: &[(i64, i64)]) -> Result<usize> {
        let tx = self.app.transaction()?;
        let mut updated = 0usize;
        {
            let mut stmt = tx.prepare(
                "UPDATE meta_object SET z=?1 \
                 WHERE id=?2 AND part_id IN (SELECT id FROM meta_part WHERE layout_id=?3)",
            )?;
            for &(id, z) in items {
                updated += stmt.execute(params![z, id, layout_id])?;
            }
        }
        tx.commit()?;
        Ok(updated)
    }

    /// Insert one object on a part of `layout_id` (#48). **Layout-scoped**: the
    /// part must belong to the layout, otherwise this is a no-op returning `None`
    /// (so a stale/forged part id can't graft an object onto a foreign layout,
    /// mirroring the geometry commands' scoping). `z` defaults to 0 and
    /// `read_only` to false; the new object owns the highest id, so by the
    /// `(z, id)` paint order it lands in front. Returns the new object id.
    pub fn create_object(&self, layout_id: i64, o: &NewObject) -> Result<Option<i64>> {
        if !self.part_in_layout(o.part_id, layout_id)? {
            return Ok(None);
        }
        self.app.execute(
            "INSERT INTO meta_object(part_id, kind, x, y, w, h, binding, content, props, parent_object_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                o.part_id,
                o.kind.as_str(),
                o.x,
                o.y,
                o.w,
                o.h,
                o.binding,
                o.content,
                o.props,
                o.parent_object_id
            ],
        )?;
        Ok(Some(self.app.last_insert_rowid()))
    }

    /// Place a value `field` object together with its separate caption `text`
    /// label (#60) — the same pairing `generate_default_form` emits, but at an
    /// arbitrary drop point. Atomic (both or neither). Layout-scoped like
    /// [`Solution::create_object`]; returns `(label_id, field_id)` or `None` if the
    /// part isn't in the layout.
    ///
    /// Label placement depends on containment:
    /// - a top-level field's caption sits to the LEFT of the value on the same row
    ///   (clamped to the band origin) — the standard field/label pairing;
    /// - a portal COLUMN's caption (#169) is a single column HEADER: it sits
    ///   directly ABOVE the column value, spanning the column width, so a portal's
    ///   authored labels read as one top header row over the repeating value rows
    ///   rather than a per-row left caption.
    ///
    /// `parent` (#168/#169, Model B) is the owning portal when the pair is placed
    /// as a portal COLUMN; both the label and the value become children of it, so
    /// they cascade-delete and move with the portal. `None` for a top-level place.
    pub fn create_field_object(
        &mut self,
        layout_id: i64,
        part_id: i64,
        binding: &str,
        label: &str,
        x: i64,
        y: i64,
        w: i64,
        h: i64,
        parent: Option<i64>,
    ) -> Result<Option<(i64, i64)>> {
        if !self.part_in_layout(part_id, layout_id)? {
            return Ok(None);
        }
        // A portal column's label is a top header (above the value, column width);
        // a top-level field's label is a left caption (to the left, fixed 72 wide).
        let (label_x, label_y, label_w) = match parent {
            Some(_) => (x, (y - h).max(0), w),
            None => ((x - 80).max(0), y, 72),
        };
        let tx = self.app.transaction()?;
        tx.execute(
            "INSERT INTO meta_object(part_id, kind, x, y, w, h, content, parent_object_id) \
             VALUES (?1, 'text', ?2, ?3, ?4, ?5, ?6, ?7)",
            params![part_id, label_x, label_y, label_w, h, label, parent],
        )?;
        let label_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO meta_object(part_id, kind, x, y, w, h, binding, parent_object_id) \
             VALUES (?1, 'field', ?2, ?3, ?4, ?5, ?6, ?7)",
            params![part_id, x, y, w, h, binding, parent],
        )?;
        let field_id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(Some((label_id, field_id)))
    }

    /// Re-insert previously deleted objects at their EXACT original ids, atomically
    /// (#84 — identity-preserving undo of a delete / redo of a create). Layout-
    /// scoped like [`Solution::create_object`]. A plain explicit-id INSERT (never
    /// `OR REPLACE`): an id already in use — reused by an intervening create under
    /// the plain-rowid schema (`0001_init_meta.sql`, `INTEGER PRIMARY KEY` without
    /// `AUTOINCREMENT`) — is reported as [`RestoreResult::IdInUse`] rather than
    /// silently clobbering a live row. A foreign/unknown part is
    /// [`RestoreResult::PartNotFound`]. Either rejection rolls the whole batch back
    /// (the dropped transaction rolls back automatically on early return), so a
    /// field+label pair never half-restores.
    pub fn restore_objects(
        &mut self,
        layout_id: i64,
        objs: &[RestoreObject],
    ) -> Result<RestoreResult> {
        let tx = self.app.transaction()?;
        // A restored column may reference a parent restored later in the SAME
        // batch (#168/#169). Defer the self-FK check to COMMIT so batch order
        // doesn't matter; the pragma resets itself at the end of this transaction.
        tx.pragma_update(None, "defer_foreign_keys", true)?;
        for o in objs {
            let part_ok: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM meta_part WHERE id = ?1 AND layout_id = ?2)",
                params![o.part_id, layout_id],
                |r| r.get(0),
            )?;
            if !part_ok {
                return Ok(RestoreResult::PartNotFound);
            }
            let id_taken: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM meta_object WHERE id = ?1)",
                params![o.id],
                |r| r.get(0),
            )?;
            if id_taken {
                return Ok(RestoreResult::IdInUse);
            }
            tx.execute(
                "INSERT INTO meta_object(id, part_id, kind, x, y, w, h, z, read_only, binding, content, props, parent_object_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    o.id,
                    o.part_id,
                    o.kind.as_str(),
                    o.x,
                    o.y,
                    o.w,
                    o.h,
                    o.z,
                    o.read_only as i64,
                    o.binding,
                    o.content,
                    o.props,
                    o.parent_object_id
                ],
            )?;
        }
        tx.commit()?;
        Ok(RestoreResult::Restored)
    }

    /// Delete an object from a layout (#48) — the undo of a create, and the Create
    /// zone's delete. **Layout-scoped**, so a foreign/unknown id is a no-op.
    /// Returns the number of rows removed (`0` ⇒ no such object in that layout).
    ///
    /// Deleting a portal CASCADES to its column children via the
    /// `parent_object_id` self-FK (`ON DELETE CASCADE`, #168/#169); the returned
    /// count is the direct row (`1`), not the cascaded children.
    pub fn delete_object(&self, layout_id: i64, object_id: i64) -> Result<usize> {
        let n = self.app.execute(
            "DELETE FROM meta_object \
             WHERE id=?1 AND part_id IN (SELECT id FROM meta_part WHERE layout_id=?2)",
            params![object_id, layout_id],
        )?;
        if n > 0 {
            self.delete_degenerate_object_groups(layout_id)?;
        }
        Ok(n)
    }

    /// Delete several objects atomically (#48 multi-delete/cut) — the bulk
    /// sibling of [`Solution::delete_object`], mirroring
    /// [`Solution::set_objects_geometry`]: one transaction, each DELETE
    /// **layout-scoped** so foreign/unknown ids are no-ops. Returns the total
    /// number of rows removed; degenerate groups are pruned once at the end.
    pub fn delete_objects(&mut self, layout_id: i64, object_ids: &[i64]) -> Result<usize> {
        let tx = self.app.transaction()?;
        let mut removed = 0usize;
        {
            let mut stmt = tx.prepare(
                "DELETE FROM meta_object \
                 WHERE id=?1 AND part_id IN (SELECT id FROM meta_part WHERE layout_id=?2)",
            )?;
            for &id in object_ids {
                removed += stmt.execute(params![id, layout_id])?;
            }
        }
        tx.commit()?;
        if removed > 0 {
            self.delete_degenerate_object_groups(layout_id)?;
        }
        Ok(removed)
    }

    /// Persist an object's appearance bag (#49) — the Style zone commits the
    /// opaque `props` JSON through this. **Layout-scoped** like the geometry
    /// commands; returns the rows updated (`0` ⇒ no such object in that layout).
    /// The server re-derives the shape style from these keys on the next read, so
    /// the write is authoritative.
    pub fn set_object_props(&self, layout_id: i64, object_id: i64, props: &str) -> Result<usize> {
        let n = self.app.execute(
            "UPDATE meta_object SET props=?1 \
             WHERE id=?2 AND part_id IN (SELECT id FROM meta_part WHERE layout_id=?3)",
            params![props, object_id, layout_id],
        )?;
        Ok(n)
    }

    /// Persist a binding dot-path on a field or portal object, scoped to its
    /// owning layout. The caller supplies the already validated dot-path: a
    /// field-value path (`Table.Field`) for a `field`, or a declared relationship
    /// route path for a `portal` (#168 — the anchor rides the same slot). The
    /// kind guard keeps text/shape objects (which have no data binding) untouched.
    pub fn set_object_binding(
        &self,
        layout_id: i64,
        object_id: i64,
        binding: &str,
    ) -> Result<usize> {
        let n = self.app.execute(
            "UPDATE meta_object SET binding=?1 \
             WHERE id=?2 AND kind IN ('field', 'portal') \
               AND part_id IN (SELECT id FROM meta_part WHERE layout_id=?3)",
            params![binding, object_id, layout_id],
        )?;
        Ok(n)
    }

    /// Persist a text object's static content, scoped to its owning layout.
    pub fn set_object_content(
        &self,
        layout_id: i64,
        object_id: i64,
        content: &str,
    ) -> Result<usize> {
        let n = self.app.execute(
            "UPDATE meta_object SET content=?1 \
             WHERE id=?2 AND kind='text' \
               AND part_id IN (SELECT id FROM meta_part WHERE layout_id=?3)",
            params![content, object_id, layout_id],
        )?;
        Ok(n)
    }

    /// Persist an object's Browse editability flag, scoped to its owning layout.
    pub fn set_object_read_only(
        &self,
        layout_id: i64,
        object_id: i64,
        read_only: bool,
    ) -> Result<usize> {
        let n = self.app.execute(
            "UPDATE meta_object SET read_only=?1 \
             WHERE id=?2 AND part_id IN (SELECT id FROM meta_part WHERE layout_id=?3)",
            params![if read_only { 1 } else { 0 }, object_id, layout_id],
        )?;
        Ok(n)
    }

    /// Whether `part_id` belongs to `layout_id` — the scoping guard the create
    /// commands share with the geometry commands' `part_id IN (…)` subquery.
    fn part_in_layout(&self, part_id: i64, layout_id: i64) -> Result<bool> {
        Ok(self.app.query_row(
            "SELECT EXISTS(SELECT 1 FROM meta_part WHERE id=?1 AND layout_id=?2)",
            params![part_id, layout_id],
            |r| r.get(0),
        )?)
    }
}
