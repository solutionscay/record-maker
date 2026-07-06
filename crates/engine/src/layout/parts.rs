//! Part (band) CRUD — the `impl Solution` block for `meta_part` reads and
//! writes. The part-kind legality rules (what may be created / converted, and
//! where a new band inserts) live in [`part_rules`](super::part_rules) as pure
//! functions; the methods here own the DB reads/writes around them.

use anyhow::{Result, bail};
use rusqlite::params;

use super::{PartKind, PartMeta, part_rules};
use crate::Solution;

impl Solution {
    /// Parts of a layout, stacked in `position` order (#25). An unrecognised
    /// stored `kind` falls back to `Body` (mirrors `FieldMeta`'s lenient parse).
    pub fn parts(&self, layout_id: i64) -> Result<Vec<PartMeta>> {
        let mut stmt = self.app.prepare(
            "SELECT id, layout_id, kind, height, position, props FROM meta_part \
             WHERE layout_id=?1 ORDER BY position, id",
        )?;
        let rows = stmt.query_map(params![layout_id], |r| {
            let kind_s: String = r.get(2)?;
            Ok(PartMeta {
                id: r.get(0)?,
                layout_id: r.get(1)?,
                kind: PartKind::parse(&kind_s).unwrap_or(PartKind::Body),
                height: r.get(3)?,
                position: r.get(4)?,
                props: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Create a band under the structural part rules: a layout has one body
    /// and at most one header/footer; subsummaries can repeat; grand summaries
    /// can appear once before and once after the body. The chosen position keeps
    /// header/body/footer in their structural slots and places summary bands
    /// around the body instead of blindly appending.
    pub fn create_part(&self, layout_id: i64, kind: PartKind, height: i64) -> Result<i64> {
        self.reject_form_summary(layout_id, kind)?;
        let parts = self.parts(layout_id)?;
        part_rules::validate_part_create(&parts, kind)?;
        let position = part_rules::insertion_position(&parts, kind);
        self.shift_part_positions(layout_id, position)?;
        self.app.execute(
            "INSERT INTO meta_part(layout_id, kind, height, position) VALUES (?1, ?2, ?3, ?4)",
            params![layout_id, kind.as_str(), height, position],
        )?;
        Ok(self.app.last_insert_rowid())
    }

    /// Read one part by id, scoped to `layout_id`. Returns `None` for an
    /// unknown/foreign id.
    pub fn part_by_id(&self, layout_id: i64, part_id: i64) -> Result<Option<PartMeta>> {
        let mut stmt = self.app.prepare(
            "SELECT id, layout_id, kind, height, position, props FROM meta_part \
             WHERE id=?1 AND layout_id=?2",
        )?;
        let mut rows = stmt.query_map(params![part_id, layout_id], |r| {
            let kind_s: String = r.get(2)?;
            Ok(PartMeta {
                id: r.get(0)?,
                layout_id: r.get(1)?,
                kind: PartKind::parse(&kind_s).unwrap_or(PartKind::Body),
                height: r.get(3)?,
                position: r.get(4)?,
                props: r.get(5)?,
            })
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Persist a part's band height, scoped to its layout. Returns the number of
    /// rows updated (`0` ⇒ no such part in that layout).
    pub fn set_part_height(&self, layout_id: i64, part_id: i64, height: i64) -> Result<usize> {
        let n = self.app.execute(
            "UPDATE meta_part SET height=?1 WHERE id=?2 AND layout_id=?3",
            params![height, part_id, layout_id],
        )?;
        Ok(n)
    }

    /// Persist a part's appearance bag (#49, Issue 7) — the band's opaque `props`
    /// JSON, mirroring [`Solution::set_object_props`]. Layout-scoped like the other
    /// part commands; returns the rows updated (`0` ⇒ no such part in that layout).
    /// The server re-derives the band's fill from these keys on the next read, so
    /// the write is authoritative and Browse reflects it.
    pub fn set_part_props(&self, layout_id: i64, part_id: i64, props: &str) -> Result<usize> {
        let n = self.app.execute(
            "UPDATE meta_part SET props=?1 WHERE id=?2 AND layout_id=?3",
            params![props, part_id, layout_id],
        )?;
        Ok(n)
    }

    /// Persist a part's kind, scoped to its layout. Returns the number of rows
    /// updated (`0` ⇒ no such part in that layout).
    pub fn set_part_kind(&self, layout_id: i64, part_id: i64, kind: PartKind) -> Result<usize> {
        let Some(current) = self.part_by_id(layout_id, part_id)? else {
            return Ok(0);
        };
        if current.kind == PartKind::Body && kind != PartKind::Body {
            bail!("a layout must keep exactly one body part");
        }
        // The header and footer are structural anchors — top and bottom of the
        // layout. Converting one into a summary would strand that summary at the
        // very top or bottom, the same invariant `move_part` enforces (a summary
        // can never rise above the header or sink below the footer). Since
        // `set_part_kind` never repositions, block the conversion outright.
        if matches!(kind, PartKind::SubSummary | PartKind::GrandSummary)
            && matches!(current.kind, PartKind::Header | PartKind::Footer)
        {
            bail!("the header and footer cannot become summary parts");
        }
        self.reject_form_summary(layout_id, kind)?;
        let parts = self.parts(layout_id)?;
        part_rules::validate_part_kind_change(&parts, part_id, kind)?;
        let n = self.app.execute(
            "UPDATE meta_part SET kind=?1 WHERE id=?2 AND layout_id=?3",
            params![kind.as_str(), part_id, layout_id],
        )?;
        Ok(n)
    }

    /// Delete a part from a layout. Child objects are removed by the schema's
    /// cascading foreign key. Returns the number of parts removed.
    pub fn delete_part(&self, layout_id: i64, part_id: i64) -> Result<usize> {
        if matches!(
            self.part_by_id(layout_id, part_id)?.map(|p| p.kind),
            Some(PartKind::Body)
        ) {
            bail!("the body part cannot be deleted");
        }
        let n = self.app.execute(
            "DELETE FROM meta_part WHERE id=?1 AND layout_id=?2",
            params![part_id, layout_id],
        )?;
        Ok(n)
    }

    /// Move a summary band up or down within its layout, staying strictly between
    /// the header and footer (Issue 4). **Only** `SubSummary`/`GrandSummary` parts
    /// move — any other target is a no-op returning `0`. The band swaps its
    /// `position` with the adjacent part in the requested direction (previous when
    /// `up`, next when down), but the move is refused (no-op, `0`) when there is no
    /// neighbour, or the neighbour is the `Header` (moving up) or `Footer` (moving
    /// down) — so a summary can never rise above the header or sink below the
    /// footer. Layout-scoped like the other part commands; the swap runs in a
    /// transaction so positions never half-update. Returns the rows changed (`0` =
    /// no move, `2` = swapped).
    pub fn move_part(&mut self, layout_id: i64, part_id: i64, up: bool) -> Result<usize> {
        let parts = self.parts(layout_id)?; // ordered by (position, id)
        let Some(idx) = parts.iter().position(|p| p.id == part_id) else {
            return Ok(0);
        };
        let part = &parts[idx];
        if !matches!(part.kind, PartKind::SubSummary | PartKind::GrandSummary) {
            return Ok(0);
        }
        let neighbor = if up {
            if idx == 0 {
                return Ok(0);
            }
            &parts[idx - 1]
        } else {
            if idx + 1 >= parts.len() {
                return Ok(0);
            }
            &parts[idx + 1]
        };
        if (up && neighbor.kind == PartKind::Header) || (!up && neighbor.kind == PartKind::Footer) {
            return Ok(0);
        }
        let (a_id, a_pos) = (part.id, part.position);
        let (b_id, b_pos) = (neighbor.id, neighbor.position);
        let tx = self.app.transaction()?;
        tx.execute(
            "UPDATE meta_part SET position=?1 WHERE id=?2 AND layout_id=?3",
            params![b_pos, a_id, layout_id],
        )?;
        tx.execute(
            "UPDATE meta_part SET position=?1 WHERE id=?2 AND layout_id=?3",
            params![a_pos, b_id, layout_id],
        )?;
        tx.commit()?;
        Ok(2)
    }

    /// Form layouts allow only header/body/footer — summary bands are a List/Table
    /// feature (Issue 3). Reject creating or converting to a summary when the
    /// owning layout renders as a form (defense in depth; the client greys these
    /// options out too).
    fn reject_form_summary(&self, layout_id: i64, kind: PartKind) -> Result<()> {
        if matches!(kind, PartKind::SubSummary | PartKind::GrandSummary) {
            if let Some(lay) = self.layout_by_id(layout_id)? {
                if lay.view == "form" {
                    bail!("a form layout allows only header, body, and footer parts");
                }
            }
        }
        Ok(())
    }

    fn shift_part_positions(&self, layout_id: i64, from: i64) -> Result<()> {
        self.app.execute(
            "UPDATE meta_part SET position = position + 1 WHERE layout_id=?1 AND position >= ?2",
            params![layout_id, from],
        )?;
        Ok(())
    }
}
