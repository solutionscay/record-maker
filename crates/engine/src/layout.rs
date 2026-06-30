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

use anyhow::Result;
use rusqlite::{params, Connection, Transaction};

use crate::Solution;

/// Metadata for a layout. A layout binds to a primary table (ADR-0003) and is
/// rendered live by Browse and edited by Layout mode (ADR-0005).
#[derive(Debug, Clone)]
pub struct LayoutMeta {
    pub id: i64,
    pub name: String,
    pub table_id: i64,
    pub view: String,
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
///
/// The closed set the canvas and engine agree on (#43/#60); stored as text in
/// `meta_object.kind`. Further kinds (button / portal / image) join this enum when
/// their rendering lands, so the set stays exactly what can render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Field,
    Text,
    Rect,
    Line,
    Ellipse,
}

impl ObjectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ObjectKind::Field => "field",
            ObjectKind::Text => "text",
            ObjectKind::Rect => "rect",
            ObjectKind::Line => "line",
            ObjectKind::Ellipse => "ellipse",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "field" => ObjectKind::Field,
            "text" => ObjectKind::Text,
            "rect" => ObjectKind::Rect,
            "line" => ObjectKind::Line,
            "ellipse" => ObjectKind::Ellipse,
            _ => return None,
        })
    }

    /// Whether this kind is data-bound (resolves a `binding` to a live value).
    pub fn is_field(self) -> bool {
        matches!(self, ObjectKind::Field)
    }

    /// Whether this kind is a drawn shape (rendered from `props`, no data/text).
    pub fn is_shape(self) -> bool {
        matches!(self, ObjectKind::Rect | ObjectKind::Line | ObjectKind::Ellipse)
    }
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
}

impl Solution {
    /// All layouts, ordered by name.
    pub fn layouts(&self) -> Result<Vec<LayoutMeta>> {
        let mut stmt = self
            .app
            .prepare("SELECT id, name, table_id, view FROM meta_layout ORDER BY name, id")?;
        let rows = stmt.query_map([], |r| {
            Ok(LayoutMeta {
                id: r.get(0)?,
                name: r.get(1)?,
                table_id: r.get(2)?,
                view: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Every layout bound to `table_id`, ordered by id (#57). A table carries one
    /// layout **per view** (form/list/table) — independent design surfaces that
    /// happen to bind the same table — so this returns the per-view siblings.
    pub fn layouts_for_table(&self, table_id: i64) -> Result<Vec<LayoutMeta>> {
        let mut stmt = self
            .app
            .prepare("SELECT id, name, table_id, view FROM meta_layout WHERE table_id=?1 ORDER BY id")?;
        let rows = stmt.query_map(params![table_id], |r| {
            Ok(LayoutMeta {
                id: r.get(0)?,
                name: r.get(1)?,
                table_id: r.get(2)?,
                view: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Parts of a layout, stacked in `position` order (#25). An unrecognised
    /// stored `kind` falls back to `Body` (mirrors `FieldMeta`'s lenient parse).
    pub fn parts(&self, layout_id: i64) -> Result<Vec<PartMeta>> {
        let mut stmt = self.app.prepare(
            "SELECT id, layout_id, kind, height, position FROM meta_part \
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
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Objects on a part, in **stacking order** — back→front by `(z, id)` so
    /// overlapping objects paint deterministically (#25/#43). An unrecognised
    /// stored `kind` falls back to `Text` (rendered, never editable).
    pub fn objects(&self, part_id: i64) -> Result<Vec<ObjectMeta>> {
        let mut stmt = self.app.prepare(
            "SELECT id, part_id, kind, x, y, w, h, z, read_only, binding, content, props \
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
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Read one object by id, **scoped** to `layout_id` (the part must belong to
    /// the layout). Returns `None` for an unknown/foreign id. Used after a props
    /// edit to re-derive that object's shape style server-side (#49).
    pub fn object_by_id(&self, layout_id: i64, object_id: i64) -> Result<Option<ObjectMeta>> {
        let mut stmt = self.app.prepare(
            "SELECT id, part_id, kind, x, y, w, h, z, read_only, binding, content, props \
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
            "INSERT INTO meta_object(part_id, kind, x, y, w, h, binding, content, props) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![o.part_id, o.kind.as_str(), o.x, o.y, o.w, o.h, o.binding, o.content, o.props],
        )?;
        Ok(Some(self.app.last_insert_rowid()))
    }

    /// Place a value `field` object together with its separate caption `text`
    /// label (#60) — the same pairing `generate_default_form` emits, but at an
    /// arbitrary drop point. The label sits to the left of the value on the same
    /// row (clamped to the band origin). Atomic (both or neither). Layout-scoped
    /// like [`Solution::create_object`]; returns `(label_id, field_id)` or `None`
    /// if the part isn't in the layout.
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
    ) -> Result<Option<(i64, i64)>> {
        if !self.part_in_layout(part_id, layout_id)? {
            return Ok(None);
        }
        let label_x = (x - 80).max(0);
        let tx = self.app.transaction()?;
        tx.execute(
            "INSERT INTO meta_object(part_id, kind, x, y, w, h, content) \
             VALUES (?1, 'text', ?2, ?3, 72, ?4, ?5)",
            params![part_id, label_x, y, h, label],
        )?;
        let label_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO meta_object(part_id, kind, x, y, w, h, binding) \
             VALUES (?1, 'field', ?2, ?3, ?4, ?5, ?6)",
            params![part_id, x, y, w, h, binding],
        )?;
        let field_id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(Some((label_id, field_id)))
    }

    /// Append a band to a layout (#48). The new part stacks below the others
    /// (`position = max(position) + 1`). Returns the new part id.
    pub fn create_part(&self, layout_id: i64, kind: PartKind, height: i64) -> Result<i64> {
        let position: i64 = self.app.query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM meta_part WHERE layout_id=?1",
            params![layout_id],
            |r| r.get(0),
        )?;
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
            "SELECT id, layout_id, kind, height, position FROM meta_part \
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

    /// Persist a part's kind, scoped to its layout. Returns the number of rows
    /// updated (`0` ⇒ no such part in that layout).
    pub fn set_part_kind(&self, layout_id: i64, part_id: i64, kind: PartKind) -> Result<usize> {
        let n = self.app.execute(
            "UPDATE meta_part SET kind=?1 WHERE id=?2 AND layout_id=?3",
            params![kind.as_str(), part_id, layout_id],
        )?;
        Ok(n)
    }

    /// Delete a part from a layout. Child objects are removed by the schema's
    /// cascading foreign key. Returns the number of parts removed.
    pub fn delete_part(&self, layout_id: i64, part_id: i64) -> Result<usize> {
        let n = self.app.execute(
            "DELETE FROM meta_part WHERE id=?1 AND layout_id=?2",
            params![part_id, layout_id],
        )?;
        Ok(n)
    }

    /// Delete an object from a layout (#48) — the undo of a create, and the Create
    /// zone's delete. **Layout-scoped**, so a foreign/unknown id is a no-op.
    /// Returns the number of rows removed (`0` ⇒ no such object in that layout).
    pub fn delete_object(&self, layout_id: i64, object_id: i64) -> Result<usize> {
        let n = self.app.execute(
            "DELETE FROM meta_object \
             WHERE id=?1 AND part_id IN (SELECT id FROM meta_part WHERE layout_id=?2)",
            params![object_id, layout_id],
        )?;
        Ok(n)
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

    /// Whether `part_id` belongs to `layout_id` — the scoping guard the create
    /// commands share with the geometry commands' `part_id IN (…)` subquery.
    fn part_in_layout(&self, part_id: i64, layout_id: i64) -> Result<bool> {
        Ok(self.app.query_row(
            "SELECT EXISTS(SELECT 1 FROM meta_part WHERE id=?1 AND layout_id=?2)",
            params![part_id, layout_id],
            |r| r.get(0),
        )?)
    }

    /// Look up a single layout by id.
    pub fn layout_by_id(&self, id: i64) -> Result<Option<LayoutMeta>> {
        let mut stmt = self
            .app
            .prepare("SELECT id, name, table_id, view FROM meta_layout WHERE id=?1")?;
        let mut rows = stmt.query_map(params![id], |r| {
            Ok(LayoutMeta {
                id: r.get(0)?,
                name: r.get(1)?,
                table_id: r.get(2)?,
                view: r.get(3)?,
            })
        })?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }
}

/// Create a default Form layout for a freshly-defined table, inside the caller's
/// transaction (so table + layout are atomic). One meta_layout (view='form'), one
/// body meta_part, and — per field — TWO objects stacked down the body (#60): a
/// `text` label (its `content` = the field name) and, beside it, a value `field`
/// object bound `<TableName>.<FieldName>` (the frozen binding contract). The label
/// is independent: it renders the caption while the field renders the value only.
/// The label is inserted first so it owns the lower id (paints behind / reads
/// left-to-right). Returns the new layout id. (#21/#60)
pub(crate) fn generate_default_form(
    tx: &Transaction<'_>,
    table_id: i64,
    table_name: &str,
    fields: &[(i64, String)],
) -> Result<i64> {
    tx.execute(
        "INSERT INTO meta_layout(name, table_id, view) VALUES (?1, ?2, 'form')",
        params![table_name, table_id],
    )?;
    let layout_id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO meta_part(layout_id, kind, height, position) VALUES (?1, 'body', ?2, 0)",
        params![layout_id, 40 + fields.len() as i64 * 32],
    )?;
    let part_id = tx.last_insert_rowid();
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
    let parts: Vec<(i64, String, i64, i64)> = {
        let mut stmt = conn.prepare(
            "SELECT id, kind, height, position FROM meta_part WHERE layout_id=?1 ORDER BY position, id",
        )?;
        let rows = stmt
            .query_map(params![src_layout_id], |r| {
                Ok((r.get(0)?, r.get::<_, String>(1)?, r.get(2)?, r.get(3)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    for (src_part_id, kind, height, position) in parts {
        conn.execute(
            "INSERT INTO meta_part(layout_id, kind, height, position) VALUES (?1, ?2, ?3, ?4)",
            params![new_layout_id, kind, height, position],
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
    use crate::layout::{NewObject, ObjectKind, PartKind};
    use crate::{FieldKind, NewField, Solution};

    #[test]
    fn parts_and_objects_read_the_default_form() {
        // The default Form layout from create_table (#21) is the fixture: one
        // body part, one field object per field, bound `<Table>.<Field>`.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table(
            "Customers",
            &[
                NewField { name: "Name".into(), kind: FieldKind::Text },
                NewField { name: "Email".into(), kind: FieldKind::Text },
            ],
        )
        .unwrap();
        let lay = &s.layouts().unwrap()[0];

        let parts = s.parts(lay.id).unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].kind, PartKind::Body);
        assert!(parts[0].height > 0);

        let objs = s.objects(parts[0].id).unwrap();
        // Per field: a separate label text object + a value field object (#60).
        assert_eq!(objs.len(), 4);
        for o in &objs {
            assert_eq!(o.part_id, parts[0].id);
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
            .execute("INSERT INTO meta_table(name, phys_name) VALUES ('T','t_x')", [])
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
        ] {
            assert_eq!(ObjectKind::parse(k.as_str()), Some(k));
        }
        assert!(PartKind::parse("nope").is_none());
        assert!(ObjectKind::parse("nope").is_none());
        assert!(ObjectKind::Field.is_field() && !ObjectKind::Text.is_field());
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
            .execute("INSERT INTO meta_table(name, phys_name) VALUES ('T','t_x')", [])
            .unwrap();
        let tid = s.app.last_insert_rowid();
        s.app
            .execute("INSERT INTO meta_layout(name, table_id) VALUES ('T', ?1)", [tid])
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
        s.create_table("Customers", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let lay = s.layouts().unwrap()[0].clone();
        let part = s.parts(lay.id).unwrap()[0].clone();
        let obj_id = s.objects(part.id).unwrap()[0].id;

        // A real move updates exactly one row and round-trips.
        assert_eq!(s.set_object_geometry(lay.id, obj_id, 33, 44, 120, 30).unwrap(), 1);
        let after = &s.objects(part.id).unwrap()[0];
        assert_eq!((after.x, after.y, after.w, after.h), (33, 44, 120, 30));

        // A foreign layout id is a no-op (scoped); geometry is unchanged.
        assert_eq!(s.set_object_geometry(lay.id + 999, obj_id, 1, 1, 1, 1).unwrap(), 0);
        let still = &s.objects(part.id).unwrap()[0];
        assert_eq!((still.x, still.y, still.w, still.h), (33, 44, 120, 30));

        // An unknown object id is a no-op too.
        assert_eq!(s.set_object_geometry(lay.id, 999_999, 0, 0, 0, 0).unwrap(), 0);
    }

    #[test]
    fn set_objects_geometry_commits_group_atomically_and_scoped() {
        // #46 group transform: many objects persist in one transaction; foreign or
        // unknown ids are skipped, and the count reflects only real updates.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table(
            "Customers",
            &[
                NewField { name: "Name".into(), kind: FieldKind::Text },
                NewField { name: "Email".into(), kind: FieldKind::Text },
            ],
        )
        .unwrap();
        let lay = s.layouts().unwrap()[0].clone();
        let part = s.parts(lay.id).unwrap()[0].clone();
        let objs = s.objects(part.id).unwrap();
        let (a, b) = (objs[0].id, objs[1].id);

        // Move both, plus an unknown id that must be ignored.
        let n = s
            .set_objects_geometry(
                lay.id,
                &[(a, 10, 20, 100, 24), (b, 30, 40, 100, 24), (999_999, 0, 0, 1, 1)],
            )
            .unwrap();
        assert_eq!(n, 2, "only the two real objects update");
        let after = s.objects(part.id).unwrap();
        assert_eq!((after[0].x, after[0].y), (10, 20));
        assert_eq!((after[1].x, after[1].y), (30, 40));

        // A foreign layout id updates nothing.
        assert_eq!(s.set_objects_geometry(lay.id + 999, &[(a, 1, 1, 1, 1)]).unwrap(), 0);
        assert_eq!((s.objects(part.id).unwrap()[0].x, s.objects(part.id).unwrap()[0].y), (10, 20));
    }

    #[test]
    fn per_view_layouts_are_independent() {
        // create_table yields three per-view layouts whose parts/objects are
        // distinct rows, so editing one view never touches another (#57).
        let mut s = Solution::open_in_memory().unwrap();
        let tid = s
            .create_table("Customers", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let layouts = s.layouts_for_table(tid).unwrap();
        assert_eq!(layouts.len(), 3);
        let form = layouts.iter().find(|l| l.view == "form").unwrap();
        let list = layouts.iter().find(|l| l.view == "list").unwrap();

        let form_part = s.parts(form.id).unwrap()[0].clone();
        let list_part = s.parts(list.id).unwrap()[0].clone();
        assert_ne!(form_part.id, list_part.id, "parts are distinct rows");
        let form_obj = s.objects(form_part.id).unwrap()[0].id;
        let list_obj = s.objects(list_part.id).unwrap()[0].id;
        assert_ne!(form_obj, list_obj, "objects are distinct rows");

        // Move the Form object; the List sibling must stay put.
        s.set_object_geometry(form.id, form_obj, 99, 88, 50, 20).unwrap();
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
            .execute("INSERT INTO meta_table(name, phys_name) VALUES ('T','t_x')", [])
            .unwrap();
        let tid = s.app.last_insert_rowid();
        s.app
            .execute("INSERT INTO meta_layout(name, table_id) VALUES ('T', ?1)", [tid])
            .unwrap();
        let lid = s.app.last_insert_rowid();
        s.app
            .execute("INSERT INTO meta_part(layout_id, kind) VALUES (?1, 'body')", [lid])
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
        assert_eq!((objs[0].z, objs[0].kind, objs[0].read_only), (0, ObjectKind::Text, false));
        assert_eq!(objs[0].content.as_deref(), Some("back"));
        assert!(objs[0].binding.is_none());
        assert_eq!((objs[1].z, objs[1].kind, objs[1].read_only), (10, ObjectKind::Field, true));
        assert_eq!(objs[1].binding.as_deref(), Some("top"));
        assert!(objs[1].content.is_none());
    }

    #[test]
    fn create_object_inserts_scoped_and_round_trips_payload() {
        // #48: a shape object inserts onto a body part of the layout, carries its
        // props, and defaults to z=0 / editable. A foreign part id is a no-op.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table("Customers", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let lay = s.layouts().unwrap()[0].clone();
        let part = s.parts(lay.id).unwrap()[0].clone();
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
                },
            )
            .unwrap()
            .expect("created");
        let objs = s.objects(part.id).unwrap();
        assert_eq!(objs.len(), before + 1);
        let made = objs.iter().find(|o| o.id == id).unwrap();
        assert_eq!((made.kind, made.x, made.y, made.w, made.h, made.z), (ObjectKind::Rect, 12, 8, 64, 40, 0));
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
        };
        assert!(s.create_object(lay.id, &other).unwrap().is_none());
        assert_eq!(s.objects(part.id).unwrap().len(), before + 1, "no foreign insert");
    }

    #[test]
    fn create_field_object_spawns_label_and_value_atomically() {
        // #60: dropping a field places a value `field` plus a separate caption
        // `text` label on the same row, the label to the left of the value.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table("Customers", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let lay = s.layouts().unwrap()[0].clone();
        let part = s.parts(lay.id).unwrap()[0].clone();

        let (label_id, field_id) = s
            .create_field_object(lay.id, part.id, "Customers.Email", "Email", 120, 40, 200, 24)
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
        assert!(s
            .create_field_object(lay.id, 999_999, "Customers.Name", "Name", 0, 0, 1, 1)
            .unwrap()
            .is_none());
    }

    #[test]
    fn create_part_appends_band_at_next_position() {
        // #48: a new band stacks below the existing ones.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table("Customers", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let lay = s.layouts().unwrap()[0].clone();
        let before = s.parts(lay.id).unwrap();
        let top_pos = before.iter().map(|p| p.position).max().unwrap();

        let pid = s.create_part(lay.id, PartKind::Footer, 48).unwrap();
        let parts = s.parts(lay.id).unwrap();
        assert_eq!(parts.len(), before.len() + 1);
        let made = parts.iter().find(|p| p.id == pid).unwrap();
        assert_eq!(made.kind, PartKind::Footer);
        assert_eq!(made.height, 48);
        assert!(made.position > top_pos, "appended below");
    }

    #[test]
    fn part_height_kind_and_delete_are_layout_scoped() {
        // Part edits are scoped to their owning layout, like object geometry.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table("Customers", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let lay = s.layouts().unwrap()[0].clone();
        let part = s.parts(lay.id).unwrap()[0].clone();
        let object_id = s.objects(part.id).unwrap()[0].id;

        assert_eq!(s.set_part_height(lay.id, part.id, 180).unwrap(), 1);
        assert_eq!(s.part_by_id(lay.id, part.id).unwrap().unwrap().height, 180);
        assert_eq!(s.set_part_height(lay.id + 999, part.id, 40).unwrap(), 0);
        assert_eq!(s.part_by_id(lay.id, part.id).unwrap().unwrap().height, 180);

        assert_eq!(s.set_part_kind(lay.id, part.id, PartKind::Footer).unwrap(), 1);
        assert_eq!(s.part_by_id(lay.id, part.id).unwrap().unwrap().kind, PartKind::Footer);
        assert_eq!(s.set_part_kind(lay.id + 999, part.id, PartKind::Header).unwrap(), 0);
        assert_eq!(s.part_by_id(lay.id, part.id).unwrap().unwrap().kind, PartKind::Footer);

        assert_eq!(s.delete_part(lay.id + 999, part.id).unwrap(), 0);
        assert!(s.objects(part.id).unwrap().iter().any(|o| o.id == object_id));
        assert_eq!(s.delete_part(lay.id, part.id).unwrap(), 1);
        assert!(s.part_by_id(lay.id, part.id).unwrap().is_none());
        assert!(s.objects(part.id).unwrap().is_empty(), "child objects cascade away");
    }

    #[test]
    fn delete_object_is_scoped() {
        // #48: delete removes the row, but only when it belongs to the layout.
        let mut s = Solution::open_in_memory().unwrap();
        s.create_table("Customers", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let lay = s.layouts().unwrap()[0].clone();
        let part = s.parts(lay.id).unwrap()[0].clone();
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
        s.create_table("Customers", &[NewField { name: "Name".into(), kind: FieldKind::Text }])
            .unwrap();
        let lay = s.layouts().unwrap()[0].clone();
        let part = s.parts(lay.id).unwrap()[0].clone();
        let obj_id = s.objects(part.id).unwrap()[0].id;

        assert_eq!(s.set_object_props(lay.id, obj_id, "{\"fill\":\"#123456\"}").unwrap(), 1);
        let o = s.objects(part.id).unwrap().into_iter().find(|o| o.id == obj_id).unwrap();
        assert_eq!(o.props.as_deref(), Some("{\"fill\":\"#123456\"}"));
        // Foreign layout ⇒ no-op.
        assert_eq!(s.set_object_props(lay.id + 999, obj_id, "{}").unwrap(), 0);
    }
}
