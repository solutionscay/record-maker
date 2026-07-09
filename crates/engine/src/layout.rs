//! Layout metadata read accessors **and the structural Layout-Mode contract**
//! (#43). The canonical `LayoutMeta`/`PartMeta`/`ObjectMeta` live here and are
//! reused by every consumer (Browse rendering, mode routing, the design canvas)
//! — defined exactly once (see Build Plan: engine accessor ledger).
//!
//! # The structural contract — every property the canvas reads/writes
//!
//! A layout is `meta_layout` → ordered `meta_part` bands → each band holds
//! `meta_object` controls positioned **relative to that band**. The canvas
//! edits these and Browse renders them; this is the permanent metadata model
//! (ADR-0001/0003/0004). Appearance/styling (fill, border, fonts, colour) is a
//! *separate* contract owned by #49 and carried in [`ObjectMeta::props`]; this
//! module defines only the **structure**.
//!
//! ## Layout ([`LayoutMeta`])
//! - `table_id` — the primary table the layout binds to (ADR-0003: no table
//!   occurrences; bindings are dot-paths from this table).
//! - `view` — default Browse view: `form` | `list` | `table`.
//!
//! ## Part / band ([`PartMeta`], [`PartKind`])
//! - `kind` — `header` | `body` | `footer` | `subsummary` | `grandsummary`.
//!   Governs *where* and *how often* the band renders: header/footer/summary
//!   bands render once per page; `body` repeats once per record in List/Table.
//! - `height` — band height in pixels. **Resize semantics:** the designer sets
//!   it by dragging the band's bottom boundary in Layout mode; it cannot shrink
//!   below the bottom edge of its lowest object (content is never clipped by a
//!   resize). Stored as the authoritative height; Browse lays the band out at
//!   exactly this height.
//! - `position` — band order top→bottom within the layout (`0` = topmost).
//!
//! ## Object / control ([`ObjectMeta`], [`ObjectKind`])
//! - `kind` — `field` (data-bound, renders the value only), `text` (static label
//!   from `content`), or a shape (`rect` / `line` / `ellipse`, drawn from `props`).
//!   See [`ObjectKind`] for how each renders.
//! - `content` — the static text of a `text` object (its own slot; `binding` is
//!   data-paths only). `None` for `field`/shape objects.
//! - `x`, `y`, `w`, `h` — geometry in pixels, **relative to the owning part's
//!   top-left** (the frozen geometry contract, #25). `x`/`y` are measured from
//!   the band origin, not the page; `w`/`h` are the object's box.
//! - `z` — stacking order **within the part**, for overlapping objects. Objects
//!   paint back→front by `(z asc, id asc)` and carry an explicit CSS `z-index`,
//!   so overlap is deterministic regardless of insertion order. Higher = front.
//! - `read_only` — per-object Browse editability (#40/#43). When `true`, Browse
//!   renders the value as a non-editable display instead of an input. Default
//!   `false` (editable). Editability is the object's property, identical across
//!   Form/List — not a per-view toggle.
//! - `binding` — dot-path expression to a field (`Customers.Name`) or related
//!   field (`Invoice.bill_to.name`), resolved against the layout's table.
//! - `props` — JSON bag reserved for appearance/style and misc (#49). The
//!   structural contract does not define its shape; it round-trips opaquely.

use anyhow::{Result, bail};
use rusqlite::{Connection, Transaction, params};

use crate::Solution;

mod groups;
mod objects;
mod part_rules;
mod parts;

/// Metadata for a layout. A layout binds to a primary table (ADR-0003) and is
/// rendered live by Browse and edited by Layout mode (ADR-0005).
#[derive(Debug, Clone)]
pub struct LayoutMeta {
    pub id: i64,
    pub name: String,
    pub table_id: i64,
    pub view: String,
    /// Order in the flat Layout Manager list (#149), lowest first. Not scoped
    /// per table — every layout in the solution shares one global order.
    pub position: i64,
    /// One of a table's auto-generated Form/List/Table trio (#151). Default
    /// layouts can be enabled/disabled per view but never deleted. Custom
    /// layouts (Layout Manager "New layout") are `false`: freely deletable.
    pub is_default: bool,
    /// Whether this layout participates in Browse navigation — the sidebar
    /// picker and view toggle only surface enabled defaults (#151). Custom
    /// layouts are always stored `enabled = true`; the flag is only meaningful
    /// for the default trio.
    pub enabled: bool,
}

/// The kind of a layout part (band). Determines where the band renders and how
/// often: `Header`/`Footer`/`SubSummary`/`GrandSummary` render once per page,
/// while `Body` repeats once per record in List/Table view. The closed set the
/// canvas and engine agree on (#43); stored as text in `meta_part.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartKind {
    Header,
    Body,
    Footer,
    SubSummary,
    GrandSummary,
}

impl PartKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PartKind::Header => "header",
            PartKind::Body => "body",
            PartKind::Footer => "footer",
            PartKind::SubSummary => "subsummary",
            PartKind::GrandSummary => "grandsummary",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "header" => PartKind::Header,
            "body" => PartKind::Body,
            "footer" => PartKind::Footer,
            "subsummary" => PartKind::SubSummary,
            "grandsummary" => PartKind::GrandSummary,
            _ => return None,
        })
    }
}

/// The kind of a layout object, and how each renders (#60):
/// - `Field` — a **data-bound** control: renders the bound field's live **value
///   only** (an editable input in Browse unless the object is read-only). Its
///   caption is a *separate* `Text` object, not baked into the field.
/// - `Text` — **static** text/label content from its own `content` slot (never
///   editable). A field's label is one of these, auto-spawned beside the field.
/// - `Rect` / `Line` / `Ellipse` — **shapes**: no data, no text; drawn as a styled
///   box from `props` (fill / stroke / radius) at the object's geometry and `z`.
/// - `Portal` — a **related-list container** (#168): binds a declared relationship
///   route (its dot-path rides the `binding` slot, exactly like a field binding)
///   and renders a repeating region of the related table's records (#169). FK-first
///   — a portal only SELECTS an existing route, it never creates one.
///
/// The closed set the canvas and engine agree on (#43/#60); stored as text in
/// `meta_object.kind`. Further kinds (button / image) join this enum when their
/// rendering lands, so the set stays exactly what can render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Field,
    Text,
    Rect,
    Line,
    Ellipse,
    Portal,
}

impl ObjectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ObjectKind::Field => "field",
            ObjectKind::Text => "text",
            ObjectKind::Rect => "rect",
            ObjectKind::Line => "line",
            ObjectKind::Ellipse => "ellipse",
            ObjectKind::Portal => "portal",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "field" => ObjectKind::Field,
            "text" => ObjectKind::Text,
            "rect" => ObjectKind::Rect,
            "line" => ObjectKind::Line,
            "ellipse" => ObjectKind::Ellipse,
            "portal" => ObjectKind::Portal,
            _ => return None,
        })
    }

    /// Whether this kind is data-bound (resolves a `binding` to a live value).
    pub fn is_field(self) -> bool {
        matches!(self, ObjectKind::Field)
    }

    /// Whether this kind is a portal — a related-list container binding a
    /// relationship route (#168). Its `binding` holds the route dot-path.
    pub fn is_portal(self) -> bool {
        matches!(self, ObjectKind::Portal)
    }

    /// Whether this kind is a drawn shape (rendered from `props`, no data/text).
    pub fn is_shape(self) -> bool {
        matches!(
            self,
            ObjectKind::Rect | ObjectKind::Line | ObjectKind::Ellipse
        )
    }

    /// Every object kind, in declaration order — for building the per-kind
    /// capability table the design model exports.
    pub const ALL: [ObjectKind; 6] = [
        ObjectKind::Field,
        ObjectKind::Text,
        ObjectKind::Rect,
        ObjectKind::Line,
        ObjectKind::Ellipse,
        ObjectKind::Portal,
    ];

    /// The kind's capability record — see [`ObjectCapabilities`]. THE single
    /// per-kind capability table: the server ships it to the editor through the
    /// design model, so every "does this kind take a fill / text format /
    /// content slot / binding?" gate reads this one definition. Adding a new
    /// object kind means filling in one row here, not updating scattered
    /// predicates.
    pub fn capabilities(self) -> ObjectCapabilities {
        match self {
            ObjectKind::Field => ObjectCapabilities {
                fill: true,
                stroke: true,
                text_format: true,
                content_slot: false,
                bindable: true,
            },
            ObjectKind::Text => ObjectCapabilities {
                fill: false,
                stroke: false,
                text_format: true,
                content_slot: true,
                bindable: false,
            },
            ObjectKind::Rect | ObjectKind::Line | ObjectKind::Ellipse => ObjectCapabilities {
                fill: true,
                stroke: true,
                text_format: false,
                content_slot: false,
                bindable: false,
            },
            // A portal frame accepts a fill/border (its container box) but has no
            // text formatting or content slot. `bindable` is false: it does NOT
            // resolve a field `binding` to a value — its `binding` slot instead
            // carries a relationship route path, picked (never typed) from the
            // layout's declared routes, so the field-binding UI must not target it.
            ObjectKind::Portal => ObjectCapabilities {
                fill: true,
                stroke: true,
                text_format: false,
                content_slot: false,
                bindable: false,
            },
        }
    }
}

/// What a layout object kind can do — the per-kind capability record returned
/// by [`ObjectKind::capabilities`]. `fill`/`stroke` gate the inspector's
/// fill-and-border controls, `text_format` its font/text controls,
/// `content_slot` marks kinds carrying static text in their own `content` slot,
/// and `bindable` marks data-bound kinds that resolve a `binding` to a live
/// field value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectCapabilities {
    pub fill: bool,
    pub stroke: bool,
    pub text_format: bool,
    pub content_slot: bool,
    pub bindable: bool,
}

/// A layout part (band): header|body|footer|subsummary|grandsummary. Parts stack
/// in `position` order; an object's geometry is relative to its part (#25). See
/// the module-level contract for `height`/resize semantics.
#[derive(Debug, Clone)]
pub struct PartMeta {
    pub id: i64,
    pub layout_id: i64,
    pub kind: PartKind,
    pub height: i64,
    pub position: i64,
    /// JSON appearance bag for the band (#49) — the same opaque slot objects
    /// carry. The server derives the band's inline fill from it; the structural
    /// contract does not define its shape. `None` for an unstyled band.
    pub props: Option<String>,
}

/// An object on a part: **part-relative** geometry (the frozen geometry
/// contract) with explicit `z` stacking and a per-object `read_only` flag, plus
/// a dot-path `binding` like `Customers.Name`. The same objects are rendered
/// live by Browse (#25/#26) and edited by the canvas (#15). See the module-level
/// contract for the meaning of every field.
#[derive(Debug, Clone)]
pub struct ObjectMeta {
    pub id: i64,
    pub part_id: i64,
    pub kind: ObjectKind,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    /// Stacking order within the part; higher paints in front. See module docs.
    pub z: i64,
    /// When `true`, Browse renders a non-editable value instead of an input.
    pub read_only: bool,
    pub binding: Option<String>,
    /// Static text for a `text` object — its own slot, distinct from `binding`
    /// (which is data-paths only). `None` for `field`/shape objects.
    pub content: Option<String>,
    pub props: Option<String>,
    /// Owning container (#168/#169, Model B). `Some(portal_id)` when this object
    /// is a column field owned by a portal object; `None` for a top-level object.
    /// A self-FK with `ON DELETE CASCADE`, so a portal's columns vanish with it.
    pub parent_object_id: Option<i64>,
}

/// A durable selection/move group over existing layout objects (#75). Groups are
/// not renderable objects; every child keeps its own geometry, z, styles, and
/// owning part. Membership is one-level only: an object can belong to at most one
/// group, so regrouping selected members replaces their old groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectGroup {
    pub id: i64,
    pub object_ids: Vec<i64>,
}

/// A new object to insert on a part (#48, the Create-zone palette). Carries the
/// structural payload the caller supplies; the engine fills the interim defaults
/// (`z = 0`, `read_only = false`). `binding`/`content`/`props` follow the per-kind
/// slot rules in [`ObjectKind`] — a field sets `binding`, a text sets `content`, a
/// shape sets `props`.
#[derive(Debug, Clone)]
pub struct NewObject {
    pub part_id: i64,
    pub kind: ObjectKind,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub binding: Option<String>,
    pub content: Option<String>,
    pub props: Option<String>,
    /// Owning portal (#168/#169, Model B) when the object is placed as a column
    /// INSIDE a portal; `None` for a normal top-level create. See [`ObjectMeta`].
    pub parent_object_id: Option<i64>,
}

/// An object restored to its ORIGINAL id (identity-preserving undo of a delete /
/// redo of a create, #84). Unlike [`NewObject`] it carries `id`, `z`, and
/// `read_only` so identity and stacking survive the round-trip.
#[derive(Debug, Clone)]
pub struct RestoreObject {
    pub id: i64,
    pub part_id: i64,
    pub kind: ObjectKind,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub z: i64,
    pub read_only: bool,
    pub binding: Option<String>,
    pub content: Option<String>,
    pub props: Option<String>,
    /// Owning portal (#168/#169, Model B), preserved so a restored column keeps
    /// its containment link on undo-of-delete / redo-of-create. See [`ObjectMeta`].
    pub parent_object_id: Option<i64>,
}

/// Outcome of a [`Solution::restore_objects`] batch (#84). The batch either fully
/// restores or is rejected for a single reason — the transaction rolls back, so
/// nothing partial ever lands (a field+label pair never half-restores). The two
/// reject reasons are distinguished so the API layer maps 404 vs 409. Mirrors the
/// `Ok(None)` "part not in layout" convention of [`Solution::create_object`].
#[derive(Debug, PartialEq, Eq)]
pub enum RestoreResult {
    /// All objects re-inserted at their original ids.
    Restored,
    /// A referenced part isn't in the layout; nothing was written.
    PartNotFound,
    /// An id is already in use (reused by an intervening create); nothing written.
    IdInUse,
}

/// Column list every `layout_meta_from_row` read must SELECT, in order.
const LAYOUT_COLS: &str = "id, name, table_id, view, position, is_default, enabled";

fn layout_meta_from_row(r: &rusqlite::Row) -> rusqlite::Result<LayoutMeta> {
    Ok(LayoutMeta {
        id: r.get(0)?,
        name: r.get(1)?,
        table_id: r.get(2)?,
        view: r.get(3)?,
        position: r.get(4)?,
        is_default: r.get(5)?,
        enabled: r.get(6)?,
    })
}

impl Solution {
    /// All layouts, in the flat Layout Manager order (#149).
    pub fn layouts(&self) -> Result<Vec<LayoutMeta>> {
        let mut stmt = self.app.prepare(&format!(
            "SELECT {LAYOUT_COLS} FROM meta_layout ORDER BY position, id"
        ))?;
        let rows = stmt.query_map([], layout_meta_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Every layout bound to `table_id`, ordered by id (#57). A table carries one
    /// layout **per view** (form/list/table) — independent design surfaces that
    /// happen to bind the same table — so this returns the per-view siblings.
    /// Since #149 a table may also carry extra manager-created layouts beyond
    /// the default trio; those come back too, in id order.
    pub fn layouts_for_table(&self, table_id: i64) -> Result<Vec<LayoutMeta>> {
        let mut stmt = self.app.prepare(&format!(
            "SELECT {LAYOUT_COLS} FROM meta_layout WHERE table_id=?1 ORDER BY id"
        ))?;
        let rows = stmt.query_map(params![table_id], layout_meta_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Look up a single layout by id.
    pub fn layout_by_id(&self, id: i64) -> Result<Option<LayoutMeta>> {
        let mut stmt = self.app.prepare(&format!(
            "SELECT {LAYOUT_COLS} FROM meta_layout WHERE id=?1"
        ))?;
        let mut rows = stmt.query_map(params![id], layout_meta_from_row)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Create a new, blank (field-populated) layout for `table_id` (#149) — the
    /// Layout Manager's "New" action. Appends at the end of the global flat
    /// order. `view` must be one of `form`/`list`/`table`.
    pub fn create_layout(&mut self, table_id: i64, name: &str, view: &str) -> Result<LayoutMeta> {
        if !matches!(view, "form" | "list" | "table") {
            bail!("view must be one of form, list, table");
        }
        let Some(table) = self.table_by_id(table_id)? else {
            bail!("no table {table_id}");
        };
        // `fields` excludes the system primary key (#156), so the read-only key is
        // never auto-placed on a layout.
        let fields: Vec<(i64, String)> = self
            .fields(table_id)?
            .into_iter()
            .map(|f| (f.id, f.name))
            .collect();
        let tx = self.app.transaction()?;
        let position: i64 = tx.query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM meta_layout",
            [],
            |r| r.get(0),
        )?;
        let layout_id = generate_default_form(&tx, table_id, name, &table.name, view, &fields)?;
        tx.execute(
            "UPDATE meta_layout SET position=?1 WHERE id=?2",
            params![position, layout_id],
        )?;
        tx.commit()?;
        self.layout_by_id(layout_id)?
            .ok_or_else(|| anyhow::anyhow!("layout {layout_id} vanished after insert"))
    }

    /// Rename a layout. `None` if it doesn't exist.
    pub fn rename_layout(&mut self, id: i64, name: &str) -> Result<Option<LayoutMeta>> {
        let n = self
            .app
            .execute("UPDATE meta_layout SET name=?1 WHERE id=?2", params![name, id])?;
        if n == 0 {
            return Ok(None);
        }
        self.layout_by_id(id)
    }

    /// Delete a layout. Refuses to delete a **default** layout (#151) — the
    /// Form/List/Table trio is enable/disable-able but permanent, so a table
    /// can never be stripped of its built-in views. Only custom (Layout
    /// Manager "New layout") layouts are deletable.
    pub fn delete_layout(&mut self, id: i64) -> Result<usize> {
        let Some(layout) = self.layout_by_id(id)? else {
            return Ok(0);
        };
        if layout.is_default {
            bail!("default layouts can't be deleted — disable the view instead");
        }
        Ok(self
            .app
            .execute("DELETE FROM meta_layout WHERE id=?1", params![id])?)
    }

    /// Enable or disable a default layout view (#151). Guarded: a table must
    /// keep at least one enabled default so it never falls out of the Browse
    /// picker / view toggle. `None` if the layout doesn't exist; errors if it's
    /// a custom layout (enabled-ness is only meaningful for the default trio)
    /// or if disabling would zero out the table's last enabled default.
    pub fn set_layout_enabled(&mut self, id: i64, enabled: bool) -> Result<Option<LayoutMeta>> {
        let Some(layout) = self.layout_by_id(id)? else {
            return Ok(None);
        };
        if !layout.is_default {
            bail!("only default layouts can be enabled or disabled");
        }
        if !enabled {
            let others_enabled = self
                .layouts_for_table(layout.table_id)?
                .iter()
                .any(|l| l.is_default && l.enabled && l.id != id);
            if !others_enabled {
                bail!("a table must keep at least one enabled view");
            }
        }
        self.app.execute(
            "UPDATE meta_layout SET enabled=?1 WHERE id=?2",
            params![enabled, id],
        )?;
        self.layout_by_id(id)
    }

    /// Reorder the flat Layout Manager list (#149): `layout_ids` must include
    /// every layout in the solution, exactly once.
    pub fn reorder_layouts(&mut self, layout_ids: &[i64]) -> Result<Vec<LayoutMeta>> {
        let current = self.layouts()?;
        if current.len() != layout_ids.len() {
            bail!("layout order must include every layout exactly once");
        }
        for l in &current {
            if !layout_ids.contains(&l.id) {
                bail!("layout order must include every layout exactly once");
            }
        }
        for id in layout_ids {
            if layout_ids.iter().filter(|other| *other == id).count() != 1 {
                bail!("layout order must not contain duplicates");
            }
        }
        let tx = self.app.transaction()?;
        for (position, layout_id) in layout_ids.iter().enumerate() {
            tx.execute(
                "UPDATE meta_layout SET position=?1 WHERE id=?2",
                params![position as i64, layout_id],
            )?;
        }
        tx.commit()?;
        self.layouts()
    }
}

/// Create a blank, field-populated layout — either the default Form generated
/// for a freshly-defined table, or (#149) a Layout Manager "New" layout of any
/// view kind for an existing table — inside the caller's transaction (so it's
/// atomic with whatever else the caller is doing). One meta_layout row,
/// header/body/footer meta_parts, and — per field — TWO objects stacked down
/// the body (#60): a `text` label (its `content` = the field name) and,
/// beside it, a value `field` object bound `<TableName>.<FieldName>` (the
/// frozen binding contract) — `table_name` drives the binding regardless of
/// `layout_name`, which is only the layout's own display name. The label is
/// independent: it renders the caption while the field renders the value
/// only. The label is inserted first so it owns the lower id (paints behind /
/// reads left-to-right). Returns the new layout id. (#21/#60/#149)
pub(crate) fn generate_default_form(
    tx: &Transaction<'_>,
    table_id: i64,
    layout_name: &str,
    table_name: &str,
    view: &str,
    fields: &[(i64, String)],
) -> Result<i64> {
    tx.execute(
        "INSERT INTO meta_layout(name, table_id, view) VALUES (?1, ?2, ?3)",
        params![layout_name, table_id, view],
    )?;
    let layout_id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO meta_part(layout_id, kind, height, position) \
         VALUES (?1, 'header', 40, 0), (?1, 'body', ?2, 1), (?1, 'footer', 40, 2)",
        params![layout_id, 40 + fields.len() as i64 * 32],
    )?;
    let part_id: i64 = tx.query_row(
        "SELECT id FROM meta_part WHERE layout_id=?1 AND kind='body'",
        params![layout_id],
        |r| r.get(0),
    )?;
    for (i, (_fid, fname)) in fields.iter().enumerate() {
        let y = 16 + i as i64 * 32;
        // Caption: a separate static-text object to the left of the value.
        tx.execute(
            "INSERT INTO meta_object(part_id, kind, x, y, w, h, content) \
             VALUES (?1, 'text', 16, ?2, 72, 24, ?3)",
            params![part_id, y, fname],
        )?;
        // Value: the data-bound field, rendered value-only beside its caption.
        let binding = format!("{table_name}.{fname}");
        tx.execute(
            "INSERT INTO meta_object(part_id, kind, x, y, w, h, binding) \
             VALUES (?1, 'field', 96, ?2, 200, 24, ?3)",
            params![part_id, y, binding],
        )?;
    }
    Ok(layout_id)
}

/// Deep-copy a layout into a new one for a different `view` (#57): clones the
/// layout row, every part, and every object (geometry/z/read_only/binding/content/
/// props), so the new view starts identical to the source but is then edited
/// completely independently. Returns the new layout id. Runs inside the caller's
/// connection or transaction (`&Transaction` coerces to `&Connection`).
pub(crate) fn clone_layout(
    conn: &Connection,
    src_layout_id: i64,
    name: &str,
    table_id: i64,
    view: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO meta_layout(name, table_id, view) VALUES (?1, ?2, ?3)",
        params![name, table_id, view],
    )?;
    let new_layout_id = conn.last_insert_rowid();

    // Collect the source parts first (releasing the prepared statement) so the
    // per-part object copies below can run on the same connection.
    let parts: Vec<(i64, String, i64, i64, Option<String>)> = {
        let mut stmt = conn.prepare(
            "SELECT id, kind, height, position, props FROM meta_part WHERE layout_id=?1 ORDER BY position, id",
        )?;
        let rows = stmt
            .query_map(params![src_layout_id], |r| {
                Ok((
                    r.get(0)?,
                    r.get::<_, String>(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    for (src_part_id, kind, height, position, props) in parts {
        conn.execute(
            "INSERT INTO meta_part(layout_id, kind, height, position, props) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![new_layout_id, kind, height, position, props],
        )?;
        let new_part_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO meta_object(part_id, kind, x, y, w, h, z, read_only, binding, content, props) \
             SELECT ?1, kind, x, y, w, h, z, read_only, binding, content, props FROM meta_object WHERE part_id=?2",
            params![new_part_id, src_part_id],
        )?;
    }
    Ok(new_layout_id)
}

#[cfg(test)]
mod tests {
    use crate::PartMeta;
    use crate::layout::{NewObject, ObjectKind, PartKind, RestoreObject, RestoreResult};
    use crate::{FieldKind, NewField, Solution};

    fn body_part(s: &Solution, layout_id: i64) -> PartMeta {
        s.parts(layout_id)
            .unwrap()
            .into_iter()
            .find(|p| p.kind == PartKind::Body)
            .expect("body part")
    }

    #[test]
    fn parts_and_objects_read_the_default_form() {
        // The default Form layout from create_table (#21) is the fixture:
        // header/body/footer parts, with field objects in the body.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table(
            "Customers",
            &[
                NewField {
                    name: "Name".into(),
                    kind: FieldKind::Text,
                },
                NewField {
                    name: "Email".into(),
                    kind: FieldKind::Text,
                },
            ],
        )
        .unwrap();
        let lay = &s.layouts().unwrap()[0];

        let parts = s.parts(lay.id).unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].kind, PartKind::Header);
        assert_eq!(parts[1].kind, PartKind::Body);
        assert_eq!(parts[2].kind, PartKind::Footer);
        assert!(parts.iter().all(|p| p.height > 0));

        let body = parts.iter().find(|p| p.kind == PartKind::Body).unwrap();
        let objs = s.objects(body.id).unwrap();
        // Per field: a separate label text object + a value field object (#60).
        assert_eq!(objs.len(), 4);
        for o in &objs {
            assert_eq!(o.part_id, body.id);
            assert!(o.w > 0 && o.h > 0 && o.x >= 0 && o.y >= 0);
            // Default-form objects are editable and unstacked (the interim default).
            assert_eq!(o.z, 0);
            assert!(!o.read_only);
        }
        // Insertion is label-then-field per field, so (z,id) order is
        // [Name label, Name value, Email label, Email value].
        let (name_label, name_field, email_label, email_field) =
            (&objs[0], &objs[1], &objs[2], &objs[3]);
        // Labels are static text with the caption in `content` and no binding.
        assert_eq!(name_label.kind, ObjectKind::Text);
        assert_eq!(name_label.content.as_deref(), Some("Name"));
        assert!(name_label.binding.is_none());
        assert_eq!(email_label.kind, ObjectKind::Text);
        assert_eq!(email_label.content.as_deref(), Some("Email"));
        // Fields are value-only: a binding, no baked-in caption content.
        assert_eq!(name_field.kind, ObjectKind::Field);
        assert_eq!(name_field.binding.as_deref(), Some("Customers.Name"));
        assert!(name_field.content.is_none());
        assert_eq!(email_field.kind, ObjectKind::Field);
        assert_eq!(email_field.binding.as_deref(), Some("Customers.Email"));
        // Each label sits to the LEFT of its value on the same row; rows stack down.
        assert!(name_label.x < name_field.x, "label beside its value");
        assert_eq!(name_label.y, name_field.y, "label shares the field's row");
        assert!(name_field.y < email_field.y, "rows stacked down the body");

        // unknown ids yield empty, not error
        assert!(s.parts(999_999).unwrap().is_empty());
        assert!(s.objects(999_999).unwrap().is_empty());
    }

    #[test]
    fn layouts_empty_then_returns_inserted() {
        let s = Solution::open_in_memory().unwrap();
        assert!(s.layouts().unwrap().is_empty());

        s.app
            .execute(
                "INSERT INTO meta_table(name, phys_name) VALUES ('T','t_x')",
                [],
            )
            .unwrap();
        let tid = s.app.last_insert_rowid();
        s.app
            .execute(
                "INSERT INTO meta_layout(name, table_id, view) VALUES ('T', ?1, 'form')",
                [tid],
            )
            .unwrap();

        let ls = s.layouts().unwrap();
        assert_eq!(ls.len(), 1);
        assert_eq!(ls[0].name, "T");
        assert_eq!(ls[0].table_id, tid);
        assert_eq!(ls[0].view, "form");

        let one = s.layout_by_id(ls[0].id).unwrap().unwrap();
        assert_eq!(one.id, ls[0].id);
        assert!(s.layout_by_id(999_999).unwrap().is_none());
    }

    #[test]
    fn create_layout_appends_a_named_extra_layout_bound_to_the_right_table() {
        let mut s = Solution::open_in_memory().unwrap();
        let tid = s
            .create_table(
                "Contacts",
                &[NewField {
                    name: "Name".into(),
                    kind: FieldKind::Text,
                }],
            )
            .unwrap();

        let extra = s.create_layout(tid, "Contact Details", "form").unwrap();
        assert_eq!(extra.name, "Contact Details");
        assert_eq!(extra.table_id, tid);
        assert_eq!(extra.view, "form");

        // Bindings use the TABLE's name, not the layout's own display name.
        let objects: Vec<String> = s
            .parts(extra.id)
            .unwrap()
            .into_iter()
            .find(|p| p.kind == PartKind::Body)
            .map(|p| {
                s.objects(p.id)
                    .unwrap()
                    .into_iter()
                    .filter_map(|o| o.binding)
                    .collect()
            })
            .unwrap();
        assert_eq!(objects, vec!["Contacts.Name"]);

        // Sits after the table's own default trio in the global flat order.
        let all = s.layouts().unwrap();
        assert_eq!(all.last().unwrap().id, extra.id);

        assert!(s.create_layout(tid, "Bad", "nope").is_err());
        assert!(s.create_layout(999_999, "Bad", "form").is_err());
    }

    #[test]
    fn defaults_are_undeletable_customs_are_deletable_and_renamable() {
        let mut s = Solution::open_in_memory().unwrap();
        let tid = s.create_table("Contacts", &[]).unwrap();

        // The auto-generated trio is all default; a "New layout" is custom.
        let trio = s.layouts_for_table(tid).unwrap();
        assert_eq!(trio.len(), 3);
        assert!(trio.iter().all(|l| l.is_default && l.enabled));
        let extra = s.create_layout(tid, "Contact Details", "list").unwrap();
        assert!(!extra.is_default && extra.enabled);

        let renamed = s.rename_layout(extra.id, "Details").unwrap().unwrap();
        assert_eq!(renamed.name, "Details");
        assert!(s.rename_layout(999_999, "Nope").unwrap().is_none());

        // A custom layout deletes; a default layout never does.
        assert_eq!(s.delete_layout(extra.id).unwrap(), 1);
        assert!(s.layout_by_id(extra.id).unwrap().is_none());
        assert_eq!(s.delete_layout(extra.id).unwrap(), 0);
        for l in &trio {
            assert!(s.delete_layout(l.id).is_err(), "default {} deletable", l.view);
        }
        assert_eq!(s.layouts_for_table(tid).unwrap().len(), 3);
    }

    #[test]
    fn set_layout_enabled_toggles_defaults_and_guards_the_last_enabled() {
        let mut s = Solution::open_in_memory().unwrap();
        let tid = s.create_table("Contacts", &[]).unwrap();
        let trio = s.layouts_for_table(tid).unwrap();
        let by_view = |v: &str| trio.iter().find(|l| l.view == v).unwrap().id;
        let (form, list, table) = (by_view("form"), by_view("list"), by_view("table"));

        // Disable two of the three — fine, one stays enabled.
        assert!(!s.set_layout_enabled(form, false).unwrap().unwrap().enabled);
        assert!(!s.set_layout_enabled(list, false).unwrap().unwrap().enabled);
        // Disabling the LAST enabled default is refused.
        assert!(s.set_layout_enabled(table, false).is_err());
        assert!(s.layout_by_id(table).unwrap().unwrap().enabled);
        // Re-enabling is always fine.
        assert!(s.set_layout_enabled(form, true).unwrap().unwrap().enabled);

        // Custom layouts can't be toggled at all; unknown ids return None.
        let custom = s.create_layout(tid, "Extra", "form").unwrap();
        assert!(s.set_layout_enabled(custom.id, false).is_err());
        assert!(s.set_layout_enabled(999_999, false).unwrap().is_none());
    }

    #[test]
    fn reorder_layouts_persists_global_order_and_validates_the_set() {
        let mut s = Solution::open_in_memory().unwrap();
        let tid = s.create_table("Contacts", &[]).unwrap();
        let a = s.create_layout(tid, "A", "form").unwrap();
        let b = s.create_layout(tid, "B", "form").unwrap();

        let mut ids: Vec<i64> = s.layouts().unwrap().iter().map(|l| l.id).collect();
        assert_eq!(*ids.last().unwrap(), b.id);
        ids.reverse();
        let reordered = s.reorder_layouts(&ids).unwrap();
        assert_eq!(
            reordered.iter().map(|l| l.id).collect::<Vec<_>>(),
            ids
        );

        // Missing/duplicate ids are rejected.
        assert!(s.reorder_layouts(&[a.id]).is_err());
        assert!(s.reorder_layouts(&[a.id, a.id, b.id]).is_err());
    }

    #[test]
    fn enum_round_trip_is_total() {
        // Every contract kind survives as_str → parse unchanged.
        for k in [
            PartKind::Header,
            PartKind::Body,
            PartKind::Footer,
            PartKind::SubSummary,
            PartKind::GrandSummary,
        ] {
            assert_eq!(PartKind::parse(k.as_str()), Some(k));
        }
        for k in [
            ObjectKind::Field,
            ObjectKind::Text,
            ObjectKind::Rect,
            ObjectKind::Line,
            ObjectKind::Ellipse,
            ObjectKind::Portal,
        ] {
            assert_eq!(ObjectKind::parse(k.as_str()), Some(k));
        }
        assert!(PartKind::parse("nope").is_none());
        assert!(ObjectKind::parse("nope").is_none());
        assert!(ObjectKind::Field.is_field() && !ObjectKind::Text.is_field());
        // A portal is neither a field nor a shape; it is its own container kind.
        assert!(ObjectKind::Portal.is_portal() && !ObjectKind::Field.is_portal());
        assert!(!ObjectKind::Portal.is_field() && !ObjectKind::Portal.is_shape());
        // Shapes are the drawn kinds; field/text are not shapes.
        assert!(
            ObjectKind::Rect.is_shape()
                && ObjectKind::Line.is_shape()
                && ObjectKind::Ellipse.is_shape()
        );
        assert!(!ObjectKind::Field.is_shape() && !ObjectKind::Text.is_shape());
    }

    #[test]
    fn migration_adds_z_and_read_only_with_editable_defaults() {
        // 0002 must be applied (both migrations ran) and backfill existing-style
        // rows: an object inserted without z/read_only is unstacked + editable.
        let s = Solution::open_in_memory().unwrap();
        assert!(s.schema_version().unwrap() >= 2, "0002 applied");

        s.app
            .execute(
                "INSERT INTO meta_table(name, phys_name) VALUES ('T','t_x')",
                [],
            )
            .unwrap();
        let tid = s.app.last_insert_rowid();
        s.app
            .execute(
                "INSERT INTO meta_layout(name, table_id) VALUES ('T', ?1)",
                [tid],
            )
            .unwrap();
        let lid = s.app.last_insert_rowid();
        s.app
            .execute(
                "INSERT INTO meta_part(layout_id, kind, height) VALUES (?1, 'body', 80)",
                [lid],
            )
            .unwrap();
        let pid = s.app.last_insert_rowid();
        // Insert the pre-0002 way (no z / read_only) — the defaults must backfill.
        s.app
            .execute(
                "INSERT INTO meta_object(part_id, kind, x, y, w, h, binding) \
                 VALUES (?1, 'field', 1, 2, 3, 4, 'T.f')",
                [pid],
            )
            .unwrap();

        let o = &s.objects(pid).unwrap()[0];
        assert_eq!((o.x, o.y, o.w, o.h), (1, 2, 3, 4));
        assert_eq!(o.z, 0, "default z");
        assert!(!o.read_only, "default editable");
    }

    #[test]
    fn set_object_geometry_persists_and_is_layout_scoped() {
        // #15 round-trip primitive: geometry writes back to meta_object, scoped to
        // the owning layout so a foreign/unknown id can never mutate it.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let lay = s.layouts().unwrap()[0].clone();
        let part = body_part(&s, lay.id);
        let obj_id = s.objects(part.id).unwrap()[0].id;

        // A real move updates exactly one row and round-trips.
        assert_eq!(
            s.set_object_geometry(lay.id, obj_id, 33, 44, 120, 30)
                .unwrap(),
            1
        );
        let after = &s.objects(part.id).unwrap()[0];
        assert_eq!((after.x, after.y, after.w, after.h), (33, 44, 120, 30));

        // A foreign layout id is a no-op (scoped); geometry is unchanged.
        assert_eq!(
            s.set_object_geometry(lay.id + 999, obj_id, 1, 1, 1, 1)
                .unwrap(),
            0
        );
        let still = &s.objects(part.id).unwrap()[0];
        assert_eq!((still.x, still.y, still.w, still.h), (33, 44, 120, 30));

        // An unknown object id is a no-op too.
        assert_eq!(
            s.set_object_geometry(lay.id, 999_999, 0, 0, 0, 0).unwrap(),
            0
        );
    }

    #[test]
    fn set_objects_geometry_commits_group_atomically_and_scoped() {
        // #46 group transform: many objects persist in one transaction; foreign or
        // unknown ids are skipped, and the count reflects only real updates.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table(
            "Customers",
            &[
                NewField {
                    name: "Name".into(),
                    kind: FieldKind::Text,
                },
                NewField {
                    name: "Email".into(),
                    kind: FieldKind::Text,
                },
            ],
        )
        .unwrap();
        let lay = s.layouts().unwrap()[0].clone();
        let part = body_part(&s, lay.id);
        let objs = s.objects(part.id).unwrap();
        let (a, b) = (objs[0].id, objs[1].id);

        // Move both, plus an unknown id that must be ignored.
        let n = s
            .set_objects_geometry(
                lay.id,
                &[
                    (a, 10, 20, 100, 24),
                    (b, 30, 40, 100, 24),
                    (999_999, 0, 0, 1, 1),
                ],
            )
            .unwrap();
        assert_eq!(n, 2, "only the two real objects update");
        let after = s.objects(part.id).unwrap();
        assert_eq!((after[0].x, after[0].y), (10, 20));
        assert_eq!((after[1].x, after[1].y), (30, 40));

        // A foreign layout id updates nothing.
        assert_eq!(
            s.set_objects_geometry(lay.id + 999, &[(a, 1, 1, 1, 1)])
                .unwrap(),
            0
        );
        assert_eq!(
            (
                s.objects(part.id).unwrap()[0].x,
                s.objects(part.id).unwrap()[0].y
            ),
            (10, 20)
        );
    }

    #[test]
    fn set_objects_z_commits_group_atomically_and_scoped() {
        // #83 z-order: a Bring-to-Front/Send-to-Back re-densifies a part's stacking
        // order and persists every changed z in one transaction; foreign/unknown ids
        // are skipped and the count reflects only real updates.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table(
            "Customers",
            &[
                NewField {
                    name: "Name".into(),
                    kind: FieldKind::Text,
                },
                NewField {
                    name: "Email".into(),
                    kind: FieldKind::Text,
                },
            ],
        )
        .unwrap();
        let lay = s.layouts().unwrap()[0].clone();
        let part = body_part(&s, lay.id);
        let objs = s.objects(part.id).unwrap();
        let (a, b) = (objs[0].id, objs[1].id);

        // Re-stack both, plus an unknown id that must be ignored.
        let n = s
            .set_objects_z(lay.id, &[(a, 3), (b, 7), (999_999, 1)])
            .unwrap();
        assert_eq!(n, 2, "only the two real objects update");
        // `objects()` sorts by (z, id), so read back by id rather than position.
        let za = |s: &Solution| {
            s.objects(part.id)
                .unwrap()
                .into_iter()
                .find(|o| o.id == a)
                .unwrap()
                .z
        };
        let zb = |s: &Solution| {
            s.objects(part.id)
                .unwrap()
                .into_iter()
                .find(|o| o.id == b)
                .unwrap()
                .z
        };
        assert_eq!(za(&s), 3);
        assert_eq!(zb(&s), 7);

        // A foreign layout id updates nothing.
        assert_eq!(s.set_objects_z(lay.id + 999, &[(a, 9)]).unwrap(), 0);
        assert_eq!(za(&s), 3);
    }

    #[test]
    fn per_view_layouts_are_independent() {
        // create_table yields three per-view layouts whose parts/objects are
        // distinct rows, so editing one view never touches another (#57).
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
        let layouts = s.layouts_for_table(tid).unwrap();
        assert_eq!(layouts.len(), 3);
        let form = layouts.iter().find(|l| l.view == "form").unwrap();
        let list = layouts.iter().find(|l| l.view == "list").unwrap();

        let form_part = body_part(&s, form.id);
        let list_part = body_part(&s, list.id);
        assert_ne!(form_part.id, list_part.id, "parts are distinct rows");
        let form_obj = s.objects(form_part.id).unwrap()[0].id;
        let list_obj = s.objects(list_part.id).unwrap()[0].id;
        assert_ne!(form_obj, list_obj, "objects are distinct rows");

        // Move the Form object; the List sibling must stay put.
        s.set_object_geometry(form.id, form_obj, 99, 88, 50, 20)
            .unwrap();
        let f = &s.objects(form_part.id).unwrap()[0];
        let l = &s.objects(list_part.id).unwrap()[0];
        assert_eq!((f.x, f.y), (99, 88), "form moved");
        assert_eq!((l.x, l.y), (16, 16), "list unchanged");
    }

    #[test]
    fn objects_paint_back_to_front_and_round_trip_read_only() {
        // z-order is the overlap contract: objects() returns back→front by (z,id),
        // and the per-object read_only flag round-trips exactly.
        let s = Solution::open_in_memory().unwrap();
        s.app
            .execute(
                "INSERT INTO meta_table(name, phys_name) VALUES ('T','t_x')",
                [],
            )
            .unwrap();
        let tid = s.app.last_insert_rowid();
        s.app
            .execute(
                "INSERT INTO meta_layout(name, table_id) VALUES ('T', ?1)",
                [tid],
            )
            .unwrap();
        let lid = s.app.last_insert_rowid();
        s.app
            .execute(
                "INSERT INTO meta_part(layout_id, kind) VALUES (?1, 'body')",
                [lid],
            )
            .unwrap();
        let pid = s.app.last_insert_rowid();

        // Insert front-most first (z=10) so id order and z order disagree. The
        // field carries `binding`; the text object carries `content`.
        s.app
            .execute(
                "INSERT INTO meta_object(part_id, kind, z, read_only, binding, content) \
                 VALUES (?1, 'field', 10, 1, 'top', NULL), (?1, 'text', 0, 0, NULL, 'back')",
                [pid],
            )
            .unwrap();

        let objs = s.objects(pid).unwrap();
        assert_eq!(objs.len(), 2);
        // Lower z paints first (back); read_only, kind, and the binding/content
        // payload slots all round-trip independently.
        assert_eq!(
            (objs[0].z, objs[0].kind, objs[0].read_only),
            (0, ObjectKind::Text, false)
        );
        assert_eq!(objs[0].content.as_deref(), Some("back"));
        assert!(objs[0].binding.is_none());
        assert_eq!(
            (objs[1].z, objs[1].kind, objs[1].read_only),
            (10, ObjectKind::Field, true)
        );
        assert_eq!(objs[1].binding.as_deref(), Some("top"));
        assert!(objs[1].content.is_none());
    }

    #[test]
    fn create_object_inserts_scoped_and_round_trips_payload() {
        // #48: a shape object inserts onto a body part of the layout, carries its
        // props, and defaults to z=0 / editable. A foreign part id is a no-op.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let lay = s.layouts().unwrap()[0].clone();
        let part = body_part(&s, lay.id);
        let before = s.objects(part.id).unwrap().len();

        let id = s
            .create_object(
                lay.id,
                &NewObject {
                    part_id: part.id,
                    kind: ObjectKind::Rect,
                    x: 12,
                    y: 8,
                    w: 64,
                    h: 40,
                    binding: None,
                    content: None,
                    props: Some("{\"fill\":\"#abc\"}".into()),
                    parent_object_id: None,
                },
            )
            .unwrap()
            .expect("created");
        let objs = s.objects(part.id).unwrap();
        assert_eq!(objs.len(), before + 1);
        let made = objs.iter().find(|o| o.id == id).unwrap();
        assert_eq!(
            (made.kind, made.x, made.y, made.w, made.h, made.z),
            (ObjectKind::Rect, 12, 8, 64, 40, 0)
        );
        assert!(!made.read_only);
        assert_eq!(made.props.as_deref(), Some("{\"fill\":\"#abc\"}"));

        // A part that isn't in this layout ⇒ no-op None, no row added.
        let other = NewObject {
            part_id: 999_999,
            kind: ObjectKind::Rect,
            x: 0,
            y: 0,
            w: 1,
            h: 1,
            binding: None,
            content: None,
            props: None,
            parent_object_id: None,
        };
        assert!(s.create_object(lay.id, &other).unwrap().is_none());
        assert_eq!(
            s.objects(part.id).unwrap().len(),
            before + 1,
            "no foreign insert"
        );
    }

    #[test]
    fn create_field_object_spawns_label_and_value_atomically() {
        // #60: dropping a field places a value `field` plus a separate caption
        // `text` label on the same row, the label to the left of the value.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let lay = s.layouts().unwrap()[0].clone();
        let part = body_part(&s, lay.id);

        let (label_id, field_id) = s
            .create_field_object(
                lay.id,
                part.id,
                "Customers.Email",
                "Email",
                120,
                40,
                200,
                24,
                None,
            )
            .unwrap()
            .expect("created");
        let objs = s.objects(part.id).unwrap();
        let label = objs.iter().find(|o| o.id == label_id).unwrap();
        let field = objs.iter().find(|o| o.id == field_id).unwrap();
        assert_eq!(label.kind, ObjectKind::Text);
        assert_eq!(label.content.as_deref(), Some("Email"));
        assert!(label.binding.is_none());
        assert_eq!(field.kind, ObjectKind::Field);
        assert_eq!(field.binding.as_deref(), Some("Customers.Email"));
        assert!(field.content.is_none());
        assert!(label.x < field.x, "label sits left of the value");
        assert_eq!((field.x, field.y), (120, 40));

        // Foreign part ⇒ no-op, nothing inserted.
        assert!(
            s.create_field_object(lay.id, 999_999, "Customers.Name", "Name", 0, 0, 1, 1, None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn create_part_inserts_band_at_legal_position() {
        // #48: a new summary band inserts before the default footer so footer
        // remains the bottom singleton part.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        // Summaries are a List/Table feature (Issue 3), so exercise the List view.
        let lay = s
            .layouts()
            .unwrap()
            .into_iter()
            .find(|l| l.view == "list")
            .unwrap();
        let before = s.parts(lay.id).unwrap();
        let footer_pos = before
            .iter()
            .find(|p| p.kind == PartKind::Footer)
            .unwrap()
            .position;

        let pid = s.create_part(lay.id, PartKind::SubSummary, 48).unwrap();
        let parts = s.parts(lay.id).unwrap();
        assert_eq!(parts.len(), before.len() + 1);
        let made = parts.iter().find(|p| p.id == pid).unwrap();
        assert_eq!(made.kind, PartKind::SubSummary);
        assert_eq!(made.height, 48);
        assert!(
            made.position < footer_pos + 1,
            "inserted before shifted footer"
        );
        assert_eq!(
            parts.last().unwrap().kind,
            PartKind::Footer,
            "footer remains bottom-most"
        );
    }

    #[test]
    fn part_height_kind_and_delete_are_layout_scoped() {
        // Part edits are scoped to their owning layout, like object geometry.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        // Summaries are a List/Table feature (Issue 3), so exercise the List view.
        let lay = s
            .layouts()
            .unwrap()
            .into_iter()
            .find(|l| l.view == "list")
            .unwrap();
        let body = body_part(&s, lay.id);
        let part_id = s.create_part(lay.id, PartKind::SubSummary, 80).unwrap();
        let part = s.part_by_id(lay.id, part_id).unwrap().unwrap();

        assert_eq!(s.set_part_height(lay.id, part.id, 180).unwrap(), 1);
        assert_eq!(s.part_by_id(lay.id, part.id).unwrap().unwrap().height, 180);
        assert_eq!(s.set_part_height(lay.id + 999, part.id, 40).unwrap(), 0);
        assert_eq!(s.part_by_id(lay.id, part.id).unwrap().unwrap().height, 180);

        assert_eq!(
            s.set_part_kind(lay.id, part.id, PartKind::GrandSummary)
                .unwrap(),
            1
        );
        assert_eq!(
            s.part_by_id(lay.id, part.id).unwrap().unwrap().kind,
            PartKind::GrandSummary
        );
        assert!(
            s.set_part_kind(lay.id, body.id, PartKind::Header).is_err(),
            "body cannot be converted away"
        );
        assert_eq!(
            s.set_part_kind(lay.id + 999, part.id, PartKind::Header)
                .unwrap(),
            0
        );
        assert_eq!(
            s.part_by_id(lay.id, part.id).unwrap().unwrap().kind,
            PartKind::GrandSummary
        );

        assert_eq!(s.delete_part(lay.id + 999, part.id).unwrap(), 0);
        assert!(
            s.delete_part(lay.id, body.id).is_err(),
            "body cannot be deleted"
        );
        assert_eq!(s.delete_part(lay.id, part.id).unwrap(), 1);
        assert!(s.part_by_id(lay.id, part.id).unwrap().is_none());
        assert!(
            s.objects(part.id).unwrap().is_empty(),
            "child objects cascade away"
        );
    }

    #[test]
    fn header_and_footer_cannot_become_summaries() {
        // The structural anchors stay put: converting the header or footer into a
        // summary would strand that summary above the header / below the footer,
        // which `move_part` also forbids. `set_part_kind` never repositions, so the
        // conversion must be rejected outright.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let lay = s
            .layouts()
            .unwrap()
            .into_iter()
            .find(|l| l.view == "list")
            .unwrap();
        let parts = s.parts(lay.id).unwrap();
        let header = parts.iter().find(|p| p.kind == PartKind::Header).unwrap();
        let footer = parts.iter().find(|p| p.kind == PartKind::Footer).unwrap();

        for &target in &[PartKind::SubSummary, PartKind::GrandSummary] {
            assert!(
                s.set_part_kind(lay.id, header.id, target).is_err(),
                "header cannot become a {}",
                target.as_str()
            );
            assert!(
                s.set_part_kind(lay.id, footer.id, target).is_err(),
                "footer cannot become a {}",
                target.as_str()
            );
        }
        // The kinds are unchanged after the rejected conversions.
        assert_eq!(
            s.part_by_id(lay.id, header.id).unwrap().unwrap().kind,
            PartKind::Header
        );
        assert_eq!(
            s.part_by_id(lay.id, footer.id).unwrap().unwrap().kind,
            PartKind::Footer
        );
    }

    #[test]
    fn part_rules_reject_duplicate_singletons_and_excess_grand_summaries() {
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        // Grand summaries are a List/Table feature (Issue 3): use the List view.
        let lay = s
            .layouts()
            .unwrap()
            .into_iter()
            .find(|l| l.view == "list")
            .unwrap();

        assert!(s.create_part(lay.id, PartKind::Body, 40).is_err());
        assert!(s.create_part(lay.id, PartKind::Header, 40).is_err());
        assert!(s.create_part(lay.id, PartKind::Footer, 40).is_err());

        assert!(s.create_part(lay.id, PartKind::GrandSummary, 40).is_ok());
        assert!(s.create_part(lay.id, PartKind::GrandSummary, 40).is_ok());
        assert!(s.create_part(lay.id, PartKind::GrandSummary, 40).is_err());
    }

    #[test]
    fn delete_object_is_scoped() {
        // #48: delete removes the row, but only when it belongs to the layout.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let lay = s.layouts().unwrap()[0].clone();
        let part = body_part(&s, lay.id);
        let obj_id = s.objects(part.id).unwrap()[0].id;

        // Foreign layout ⇒ no-op.
        assert_eq!(s.delete_object(lay.id + 999, obj_id).unwrap(), 0);
        assert!(s.objects(part.id).unwrap().iter().any(|o| o.id == obj_id));
        // Real delete removes exactly one row.
        assert_eq!(s.delete_object(lay.id, obj_id).unwrap(), 1);
        assert!(!s.objects(part.id).unwrap().iter().any(|o| o.id == obj_id));
        // Deleting it again is a no-op.
        assert_eq!(s.delete_object(lay.id, obj_id).unwrap(), 0);
    }

    #[test]
    fn set_object_props_persists_scoped() {
        // #49: props write back to meta_object, layout-scoped.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let lay = s.layouts().unwrap()[0].clone();
        let part = body_part(&s, lay.id);
        let obj_id = s.objects(part.id).unwrap()[0].id;

        assert_eq!(
            s.set_object_props(lay.id, obj_id, "{\"fill\":\"#123456\"}")
                .unwrap(),
            1
        );
        let o = s
            .objects(part.id)
            .unwrap()
            .into_iter()
            .find(|o| o.id == obj_id)
            .unwrap();
        assert_eq!(o.props.as_deref(), Some("{\"fill\":\"#123456\"}"));
        // Foreign layout ⇒ no-op.
        assert_eq!(s.set_object_props(lay.id + 999, obj_id, "{}").unwrap(), 0);
    }

    #[test]
    fn selected_object_inspector_fields_persist_scoped() {
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table(
            "Customers",
            &[
                NewField {
                    name: "Name".into(),
                    kind: FieldKind::Text,
                },
                NewField {
                    name: "Email".into(),
                    kind: FieldKind::Text,
                },
            ],
        )
        .unwrap();
        let lay = s.layouts().unwrap()[0].clone();
        let part = body_part(&s, lay.id);
        let objects = s.objects(part.id).unwrap();
        let label_id = objects
            .iter()
            .find(|o| o.kind == ObjectKind::Text)
            .unwrap()
            .id;
        let field_id = objects
            .iter()
            .find(|o| o.kind == ObjectKind::Field)
            .unwrap()
            .id;

        assert_eq!(
            s.set_object_binding(lay.id, field_id, "Customers.Email")
                .unwrap(),
            1
        );
        assert_eq!(
            s.set_object_content(lay.id, label_id, "Primary email")
                .unwrap(),
            1
        );
        assert_eq!(s.set_object_read_only(lay.id, field_id, true).unwrap(), 1);

        let after = s.objects(part.id).unwrap();
        let label = after.iter().find(|o| o.id == label_id).unwrap();
        let field = after.iter().find(|o| o.id == field_id).unwrap();
        assert_eq!(label.content.as_deref(), Some("Primary email"));
        assert_eq!(field.binding.as_deref(), Some("Customers.Email"));
        assert!(field.read_only);

        assert_eq!(
            s.set_object_binding(lay.id + 999, field_id, "Customers.Name")
                .unwrap(),
            0
        );
        assert_eq!(
            s.set_object_content(lay.id + 999, label_id, "Name")
                .unwrap(),
            0
        );
        assert_eq!(
            s.set_object_read_only(lay.id + 999, field_id, false)
                .unwrap(),
            0
        );

        let unchanged = s.objects(part.id).unwrap();
        let field = unchanged.iter().find(|o| o.id == field_id).unwrap();
        assert_eq!(field.binding.as_deref(), Some("Customers.Email"));
        assert!(field.read_only);
    }

    #[test]
    fn form_layout_rejects_summary_parts_but_list_allows_them() {
        // Issue 3: a form is header/body/footer only. Creating or converting to a
        // sub/grand summary on a form is refused; List/Table still allow them.
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
        let layouts = s.layouts_for_table(tid).unwrap();
        let form = layouts.iter().find(|l| l.view == "form").unwrap();
        let list = layouts.iter().find(|l| l.view == "list").unwrap();

        // A form rejects creating a summary band of either kind.
        assert!(s.create_part(form.id, PartKind::SubSummary, 40).is_err());
        assert!(s.create_part(form.id, PartKind::GrandSummary, 40).is_err());
        // A form rejects converting an existing band to a summary.
        let form_footer = s
            .parts(form.id)
            .unwrap()
            .into_iter()
            .find(|p| p.kind == PartKind::Footer)
            .unwrap();
        assert!(
            s.set_part_kind(form.id, form_footer.id, PartKind::SubSummary)
                .is_err()
        );
        assert!(
            s.set_part_kind(form.id, form_footer.id, PartKind::GrandSummary)
                .is_err()
        );

        // A list allows creating summary bands of both kinds...
        assert!(s.create_part(list.id, PartKind::SubSummary, 40).is_ok());
        let list_grand = s.create_part(list.id, PartKind::GrandSummary, 40).unwrap();
        // ...and converting one summary kind into the other.
        assert!(
            s.set_part_kind(list.id, list_grand, PartKind::SubSummary)
                .is_ok()
        );
        // But the footer stays structural even on a list — it can't become a
        // summary (that would strand a summary below the footer).
        let list_footer = s
            .parts(list.id)
            .unwrap()
            .into_iter()
            .find(|p| p.kind == PartKind::Footer)
            .unwrap();
        assert!(
            s.set_part_kind(list.id, list_footer.id, PartKind::SubSummary)
                .is_err()
        );
    }

    #[test]
    fn move_part_reorders_summaries_and_clamps_at_boundaries() {
        // Issue 4: a summary band moves up/down but never crosses the header or
        // footer; a non-summary target is a no-op.
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
        let list = s
            .layouts_for_table(tid)
            .unwrap()
            .into_iter()
            .find(|l| l.view == "list")
            .unwrap();
        // Build: header, body, sub, grand, footer (create order places summaries
        // between the body and the footer).
        let sub = s.create_part(list.id, PartKind::SubSummary, 40).unwrap();
        let grand = s.create_part(list.id, PartKind::GrandSummary, 40).unwrap();

        let kinds = |s: &Solution| -> Vec<PartKind> {
            s.parts(list.id).unwrap().iter().map(|p| p.kind).collect()
        };
        assert_eq!(
            kinds(&s),
            vec![
                PartKind::Header,
                PartKind::Body,
                PartKind::SubSummary,
                PartKind::GrandSummary,
                PartKind::Footer,
            ]
        );

        // Move the grand summary up: it swaps with the sub summary.
        assert_eq!(s.move_part(list.id, grand, true).unwrap(), 2);
        assert_eq!(
            kinds(&s),
            vec![
                PartKind::Header,
                PartKind::Body,
                PartKind::GrandSummary,
                PartKind::SubSummary,
                PartKind::Footer,
            ]
        );

        // Move the sub summary down: it can't cross the footer — no-op.
        assert_eq!(s.move_part(list.id, sub, false).unwrap(), 0);

        // Move the grand summary up twice more: past the body, then clamp at header.
        assert_eq!(s.move_part(list.id, grand, true).unwrap(), 2); // swaps with body
        assert_eq!(kinds(&s)[1], PartKind::GrandSummary);
        assert_eq!(s.move_part(list.id, grand, true).unwrap(), 0); // header blocks it
        assert_eq!(kinds(&s)[0], PartKind::Header);
        assert_eq!(kinds(&s)[1], PartKind::GrandSummary);

        // A non-summary part (the body) never moves.
        let body = body_part(&s, list.id);
        assert_eq!(s.move_part(list.id, body.id, true).unwrap(), 0);
        // An unknown/foreign part id is a no-op.
        assert_eq!(s.move_part(list.id, 999_999, true).unwrap(), 0);
    }

    /// #168/#169 (Model B): a portal OWNS its column field objects via the
    /// `parent_object_id` self-FK. A column created under a portal reports that
    /// portal as its parent; the portal itself stays top-level. `create_object`
    /// and `create_field_object` both accept the owning parent, and it round-trips
    /// through `objects()` and `object_by_id()`.
    fn portal_fixture(s: &mut Solution) -> (i64, i64) {
        s.create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
        let lay = s.layouts().unwrap()[0].clone();
        let part = body_part(s, lay.id);
        let portal = s
            .create_object(
                lay.id,
                &NewObject {
                    part_id: part.id,
                    kind: ObjectKind::Portal,
                    x: 0,
                    y: 0,
                    w: 300,
                    h: 120,
                    binding: Some("sensors".into()),
                    content: None,
                    props: None,
                    parent_object_id: None,
                },
            )
            .unwrap()
            .expect("portal");
        (lay.id, portal)
    }

    #[test]
    fn portal_column_child_binds_to_its_parent_object() {
        let mut s = Solution::open_in_memory().unwrap();
        let (lay_id, portal) = portal_fixture(&mut s);
        let part = body_part(&s, lay_id);

        // A column field owned by the portal, bound route-relative to the
        // related table (the settled Model-B column shape).
        let col = s
            .create_object(
                lay_id,
                &NewObject {
                    part_id: part.id,
                    kind: ObjectKind::Field,
                    x: 0,
                    y: 0,
                    w: 100,
                    h: 24,
                    binding: Some("sensors.reading".into()),
                    content: None,
                    props: None,
                    parent_object_id: Some(portal),
                },
            )
            .unwrap()
            .expect("column");
        // A field+label pair placed inside the portal via create_field_object:
        // BOTH become children so they cascade and move with the portal.
        let (label2, field2) = s
            .create_field_object(
                lay_id,
                part.id,
                "sensors.name",
                "Name",
                0,
                30,
                100,
                24,
                Some(portal),
            )
            .unwrap()
            .expect("pair");

        let objs = s.objects(part.id).unwrap();
        let get = |id: i64| objs.iter().find(|o| o.id == id).unwrap();
        assert!(get(portal).parent_object_id.is_none(), "portal is top-level");
        assert_eq!(get(col).parent_object_id, Some(portal));
        assert_eq!(get(label2).parent_object_id, Some(portal));
        assert_eq!(get(field2).parent_object_id, Some(portal));
        // object_by_id carries the link too.
        assert_eq!(
            s.object_by_id(lay_id, col).unwrap().unwrap().parent_object_id,
            Some(portal)
        );
    }

    #[test]
    fn deleting_a_portal_cascades_its_column_children() {
        let mut s = Solution::open_in_memory().unwrap();
        let (lay_id, portal) = portal_fixture(&mut s);
        let part = body_part(&s, lay_id);
        let col = s
            .create_object(
                lay_id,
                &NewObject {
                    part_id: part.id,
                    kind: ObjectKind::Field,
                    x: 0,
                    y: 0,
                    w: 100,
                    h: 24,
                    binding: Some("sensors.reading".into()),
                    content: None,
                    props: None,
                    parent_object_id: Some(portal),
                },
            )
            .unwrap()
            .expect("column");

        // Deleting the portal reports ONE direct row, but cascades the child.
        assert_eq!(s.delete_object(lay_id, portal).unwrap(), 1);
        let after = s.objects(part.id).unwrap();
        assert!(!after.iter().any(|o| o.id == portal), "portal removed");
        assert!(!after.iter().any(|o| o.id == col), "column cascaded");
        assert!(
            !after.iter().any(|o| o.parent_object_id == Some(portal)),
            "no orphaned children linger"
        );
        // The default-form objects on the body are untouched by the cascade.
        assert!(after.iter().all(|o| o.parent_object_id.is_none()));
    }

    #[test]
    fn restore_preserves_and_defers_portal_parent_links() {
        let mut s = Solution::open_in_memory().unwrap();
        let (lay_id, portal) = portal_fixture(&mut s);
        let part = body_part(&s, lay_id);
        let col = s
            .create_object(
                lay_id,
                &NewObject {
                    part_id: part.id,
                    kind: ObjectKind::Field,
                    x: 5,
                    y: 6,
                    w: 100,
                    h: 24,
                    binding: Some("sensors.reading".into()),
                    content: None,
                    props: None,
                    parent_object_id: Some(portal),
                },
            )
            .unwrap()
            .expect("column");

        let snap = |s: &Solution, id: i64| {
            let o = s.object_by_id(lay_id, id).unwrap().unwrap();
            RestoreObject {
                id: o.id,
                part_id: o.part_id,
                kind: o.kind,
                x: o.x,
                y: o.y,
                w: o.w,
                h: o.h,
                z: o.z,
                read_only: o.read_only,
                binding: o.binding,
                content: o.content,
                props: o.props,
                parent_object_id: o.parent_object_id,
            }
        };
        let portal_snap = snap(&s, portal);
        let col_snap = snap(&s, col);

        // Delete the portal (cascades the column), then restore BOTH — with the
        // child listed BEFORE its parent to prove the self-FK check is deferred to
        // COMMIT rather than enforced per-INSERT.
        s.delete_object(lay_id, portal).unwrap();
        assert!(s.object_by_id(lay_id, col).unwrap().is_none());
        let res = s
            .restore_objects(lay_id, &[col_snap, portal_snap])
            .unwrap();
        assert_eq!(res, RestoreResult::Restored);

        let col_back = s.object_by_id(lay_id, col).unwrap().unwrap();
        assert_eq!(col_back.parent_object_id, Some(portal), "link preserved");
        assert_eq!((col_back.x, col_back.y), (5, 6));
        assert!(
            s.object_by_id(lay_id, portal)
                .unwrap()
                .unwrap()
                .parent_object_id
                .is_none(),
            "restored portal stays top-level"
        );
    }
}
