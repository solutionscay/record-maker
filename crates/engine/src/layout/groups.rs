//! Durable object groups (#75) — the `impl Solution` block for
//! `meta_object_group` reads and writes. Groups are membership only: child
//! objects keep their own geometry, z, styles, and owning part.

use anyhow::Result;
use rusqlite::params;

use super::ObjectGroup;
use crate::Solution;

impl Solution {
    /// Durable groups for a layout, ordered by group id and member position (#75).
    /// Groups with fewer than two live members are ignored; cleanup happens on
    /// mutation, and this read path remains lenient for older data.
    pub fn object_groups(&self, layout_id: i64) -> Result<Vec<ObjectGroup>> {
        let mut stmt = self.app.prepare(
            "SELECT g.id, m.object_id \
             FROM meta_object_group g \
             JOIN meta_object_group_member m ON m.group_id = g.id \
             JOIN meta_object o ON o.id = m.object_id \
             JOIN meta_part p ON p.id = o.part_id \
             WHERE g.layout_id=?1 AND p.layout_id=?1 \
             ORDER BY g.id, m.position, m.object_id",
        )?;
        let rows = stmt.query_map(params![layout_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?;
        let mut groups: Vec<ObjectGroup> = Vec::new();
        for row in rows {
            let (group_id, object_id) = row?;
            match groups.last_mut() {
                Some(g) if g.id == group_id => g.object_ids.push(object_id),
                _ => groups.push(ObjectGroup {
                    id: group_id,
                    object_ids: vec![object_id],
                }),
            }
        }
        groups.retain(|g| g.object_ids.len() >= 2);
        Ok(groups)
    }

    /// Create a durable group over existing objects in `layout_id` (#75). Returns
    /// `None` when fewer than two unique ids are supplied, or any id is unknown /
    /// foreign to the layout. Objects can belong to only one group; if any member
    /// was already grouped, those previous groups are removed before the new
    /// membership is inserted. That is the v1 "no nested groups" rule.
    pub fn create_object_group(
        &mut self,
        layout_id: i64,
        object_ids: &[i64],
        group_id: Option<i64>,
    ) -> Result<Option<ObjectGroup>> {
        let mut ids = Vec::new();
        for &id in object_ids {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        if ids.len() < 2 {
            return Ok(None);
        }
        for &id in &ids {
            if self.object_by_id(layout_id, id)?.is_none() {
                return Ok(None);
            }
        }

        let mut old_group_ids = Vec::new();
        {
            let mut stmt = self.app.prepare(
                "SELECT DISTINCT m.group_id \
                 FROM meta_object_group_member m \
                 JOIN meta_object_group g ON g.id = m.group_id \
                 WHERE g.layout_id=?1 AND m.object_id=?2",
            )?;
            for &id in &ids {
                let rows = stmt.query_map(params![layout_id, id], |r| r.get::<_, i64>(0))?;
                for row in rows {
                    let group_id = row?;
                    if !old_group_ids.contains(&group_id) {
                        old_group_ids.push(group_id);
                    }
                }
            }
        }

        let tx = self.app.transaction()?;
        for group_id in old_group_ids {
            tx.execute(
                "DELETE FROM meta_object_group WHERE id=?1 AND layout_id=?2",
                params![group_id, layout_id],
            )?;
        }
        let group_id = match group_id {
            Some(id) => {
                tx.execute(
                    "INSERT INTO meta_object_group(id, layout_id) VALUES (?1, ?2)",
                    params![id, layout_id],
                )?;
                id
            }
            None => {
                tx.execute(
                    "INSERT INTO meta_object_group(layout_id) VALUES (?1)",
                    params![layout_id],
                )?;
                tx.last_insert_rowid()
            }
        };
        {
            let mut stmt = tx.prepare(
                "INSERT INTO meta_object_group_member(group_id, object_id, position) \
                 VALUES (?1, ?2, ?3)",
            )?;
            for (position, id) in ids.iter().enumerate() {
                stmt.execute(params![group_id, id, position as i64])?;
            }
        }
        tx.commit()?;
        Ok(Some(ObjectGroup {
            id: group_id,
            object_ids: ids,
        }))
    }

    /// Remove a group from a layout. Child objects are untouched (#75 Ungroup).
    pub fn delete_object_group(&self, layout_id: i64, group_id: i64) -> Result<usize> {
        let n = self.app.execute(
            "DELETE FROM meta_object_group WHERE id=?1 AND layout_id=?2",
            params![group_id, layout_id],
        )?;
        Ok(n)
    }

    pub(super) fn delete_degenerate_object_groups(&self, layout_id: i64) -> Result<usize> {
        let n = self.app.execute(
            "DELETE FROM meta_object_group \
             WHERE layout_id=?1 \
               AND (SELECT count(*) FROM meta_object_group_member \
                    WHERE group_id = meta_object_group.id) < 2",
            params![layout_id],
        )?;
        Ok(n)
    }
}
