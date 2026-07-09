//! The render model: chrome, askama templates, the `PartView`/`ObjectView`
//! wire structs, and the projection helpers that resolve layout metadata +
//! record values into them. Shared by the Browse pages and the Layout-Mode
//! design model (#44 parity contract).

use std::collections::{HashMap, HashSet};

use askama::Template;
use axum::Json;
use record_maker_engine::{
    FieldKind, FieldMeta, FilterClause, FilterOp, FilterOperand, LayoutMeta, ObjectKind,
    ObjectMeta, PartKind, PartMeta, Record, RelatedFilter, Solution, TableMeta,
};

use crate::style::{object_style, parse_props, part_style, shape_style, text_style};
use crate::{format, AppError, AppResult};

/// Persistent shell context shared by every page (the chrome).
pub(crate) struct Chrome {
    pub(crate) mode: &'static str, // "browse" | "design" | "schema"
    pub(crate) layouts: Vec<LayoutLink>,
    pub(crate) current_layout: Option<i64>,
    /// Form/List/Table tabs for the Browse view toggle; empty in Layout mode.
    pub(crate) view_tabs: Vec<ViewTab>,
    /// Record-navigation flipbook for the Browse status bar; `None` elsewhere.
    pub(crate) nav: Option<Flipbook>,
    /// True when the current record is open for editing (its lock is held).
    pub(crate) editing: bool,
}

pub(crate) struct LayoutLink {
    id: i64,
    name: String,
    selected: bool,
}

/// One entry in the Browse Form/List/Table view toggle.
pub(crate) struct ViewTab {
    label: &'static str,
    href: String,
    active: bool,
}

/// Record navigation for the Browse status sidebar: first/prev/next/last over
/// the current layout's found set (#23), plus an editable position field.
/// `current` is 1-based, `0` when empty. `layout_id`/`view` back the jump form.
pub(crate) struct Flipbook {
    layout_id: i64,
    view: &'static str,
    current: i64,
    /// Physical id of the record at `current`; `None` when the found set is
    /// empty. Backs the toolbar's Delete action.
    current_id: Option<i64>,
    total: i64,
    first_href: String,
    prev_href: String,
    next_href: String,
    last_href: String,
    at_first: bool,
    at_last: bool,
}

/// Parse `?rec=N` (1-based) and clamp it into the found set (frozen #23):
/// `[1, total]`, defaulting to 1; `0` when there are no records.
pub(crate) fn clamp_rec(q: &HashMap<String, String>, total: i64) -> i64 {
    clamp_rec_n(q.get("rec").and_then(|s| s.parse::<i64>().ok()), total)
}

/// Clamp a client-sent record number into the found set (1-based, `0` when
/// empty) — the typed-body core [`clamp_rec`] parses `?rec=` into.
pub(crate) fn clamp_rec_n(rec: Option<i64>, total: i64) -> i64 {
    if total <= 0 {
        return 0;
    }
    rec.unwrap_or(1).clamp(1, total)
}

/// Build the flipbook for record `current` of `total` on `layout_id`/`view`.
/// Step links preserve the current view and stay clamped to the found set.
/// `current_id` is the physical id at `current` (for the Delete action).
pub(crate) fn flipbook(
    layout_id: i64,
    view: &'static str,
    current: i64,
    current_id: Option<i64>,
    total: i64,
) -> Flipbook {
    let href = |n: i64| format!("/browse/{layout_id}?view={view}&rec={n}");
    Flipbook {
        layout_id,
        view,
        current,
        current_id,
        total,
        first_href: href(1),
        prev_href: href((current - 1).max(1)),
        next_href: href((current + 1).min(total.max(1))),
        last_href: href(total.max(1)),
        at_first: current <= 1,
        at_last: current >= total,
    }
}

/// A table's per-view sibling layouts drawn from an already-fetched layout list,
/// in id order — the in-memory equivalent of `Solution::layouts_for_table`, so
/// chrome/stepper construction runs one `layouts()` query instead of one per
/// table.
fn layouts_for_table_in(layouts: &[LayoutMeta], table_id: i64) -> Vec<&LayoutMeta> {
    let mut siblings: Vec<&LayoutMeta> = layouts
        .iter()
        .filter(|l| l.table_id == table_id)
        .collect();
    siblings.sort_by_key(|l| l.id);
    siblings
}

/// The layout a table lands on from the sidebar picker (#151): its enabled
/// default view, preferring Form, then List, then Table. `None` if the table
/// has no enabled default (so it drops out of the picker rather than pointing
/// at a dead layout). Custom layouts are never landing handles.
fn landing_layout(layouts: &[LayoutMeta], table_id: i64) -> Option<&LayoutMeta> {
    VIEWS.iter().find_map(|&v| {
        layouts
            .iter()
            .find(|l| l.table_id == table_id && l.is_default && l.enabled && l.view == v)
    })
}

/// Build the Layout-mode stepper: prev/next steps through the **logical layouts**
/// (one per table, in picker order) while holding the current view, so the
/// designer flips between layouts the way the record stepper flips records (#57).
/// In Layout mode the pagination control navigates layouts, not records.
/// `layouts` is the full layout list (name order), fetched once by the caller
/// and shared with [`Chrome::build_with_layouts`].
pub(crate) fn layout_stepper(layouts: &[LayoutMeta], current: &LayoutMeta) -> Option<Flipbook> {
    let view = canonical_view(&current.view);
    // Each table (its Form layout is the canonical handle) → that table's layout
    // for the CURRENT view, so stepping holds the view axis steady.
    let steps: Vec<i64> = layouts
        .iter()
        .filter(|l| l.view == "form")
        .filter_map(|l| {
            layouts_for_table_in(layouts, l.table_id)
                .into_iter()
                .find(|s| s.view == view)
                .map(|s| s.id)
        })
        .collect();
    let idx = steps.iter().position(|&id| id == current.id)?;
    let href = |i: usize| format!("/design/{}", steps[i]);
    Some(Flipbook {
        layout_id: current.id,
        view,
        current: idx as i64 + 1,
        current_id: None,
        total: steps.len() as i64,
        first_href: href(0),
        prev_href: href(idx.saturating_sub(1)),
        next_href: href((idx + 1).min(steps.len() - 1)),
        last_href: href(steps.len() - 1),
        at_first: idx == 0,
        at_last: idx + 1 >= steps.len(),
    })
}

/// The three Browse views, in toggle order. The frozen `?view=` contract (#20).
const VIEWS: [&str; 3] = ["form", "list", "table"];

/// Normalise a `?view=` value to a known view, falling back to the layout's
/// stored view when `?view` is absent. Retained for the record-action handlers'
/// redirects; Browse itself now renders by the layout's own view (see
/// [`canonical_view`]), since each view is its own layout (#57).
pub(crate) fn view_param(q: &HashMap<String, String>, default: &str) -> &'static str {
    canonical_view(q.get("view").map(String::as_str).unwrap_or(default))
}

/// Normalise a stored layout `view` string to one of the three renderers. A
/// layout's view is now intrinsic — the layout id encodes the view — so Browse
/// renders by this rather than a `?view=` param (#57).
pub(crate) fn canonical_view(view: &str) -> &'static str {
    match view {
        "form" => "form",
        "list" => "list",
        _ => "table",
    }
}

/// Human label for a stored `view` (the toggle tabs + the Layout-mode status).
pub(crate) fn view_label(view: &str) -> &'static str {
    match view {
        "form" => "Form",
        "list" => "List",
        _ => "Table",
    }
}

impl Chrome {
    /// Build the shared chrome. `current` is the layout in focus (its view + table
    /// drive the toggle and picker). The picker lists one entry per table (an
    /// enabled default layout is the canonical handle, #151), and the view toggle
    /// switches among that table's enabled default view siblings.
    pub(crate) fn build(sol: &Solution, mode: &'static str, current: Option<&LayoutMeta>) -> Self {
        Self::build_with_layouts(&sol.layouts().unwrap_or_default(), mode, current)
    }

    /// [`Chrome::build`] over an already-fetched layout list, so a handler that
    /// also needs the list for the [`layout_stepper`] fetches it once.
    pub(crate) fn build_with_layouts(
        all: &[LayoutMeta],
        mode: &'static str,
        current: Option<&LayoutMeta>,
    ) -> Self {
        let current_table = current.map(|c| c.table_id);
        // Picker: one entry per table that still has an enabled default view
        // (#151). Prefer Form as the landing handle, else the first enabled
        // default in view order — so a table never drops out of the picker just
        // because its Form view got disabled (or, pre-#151, deleted).
        let mut seen_tables: Vec<i64> = Vec::new();
        let mut layouts: Vec<LayoutLink> = Vec::new();
        for l in all {
            if seen_tables.contains(&l.table_id) {
                continue;
            }
            seen_tables.push(l.table_id);
            if let Some(land) = landing_layout(all, l.table_id) {
                layouts.push(LayoutLink {
                    selected: current_table == Some(l.table_id),
                    id: land.id,
                    name: land.name.clone(),
                });
            }
        }
        // The view toggle switches among the current table's enabled default view
        // siblings only — disabled views and custom layouts never appear as tabs
        // (#151). It stays in the current mode, so Layout mode designs each view.
        let view_tabs = match current {
            Some(cur) => {
                let siblings = layouts_for_table_in(all, cur.table_id);
                VIEWS
                    .iter()
                    .filter_map(|&v| {
                        siblings
                            .iter()
                            .find(|l| l.view == v && l.is_default && l.enabled)
                            .map(|l| ViewTab {
                                label: view_label(v),
                                href: format!("/{mode}/{}", l.id),
                                active: cur.view == v,
                            })
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        Chrome {
            mode,
            layouts,
            current_layout: current.map(|c| c.id),
            view_tabs,
            nav: None,
            editing: false,
        }
    }
}

/// Resolve a layout id to its (layout, primary table). `None` if unknown.
pub(crate) fn layout_table(sol: &Solution, layout_id: i64) -> Option<(LayoutMeta, TableMeta)> {
    let lay = sol.layout_by_id(layout_id).ok().flatten()?;
    let tbl = sol.table_by_id(lay.table_id).ok().flatten()?;
    Some((lay, tbl))
}

// ---- Browse views — Table (live), Form/List placeholders until #25/#26 ----

#[derive(Template)]
#[template(path = "view_table.html")]
pub(crate) struct TableTemplate {
    pub(crate) chrome: Chrome,
    pub(crate) layout_id: i64,
    pub(crate) table: String,
    /// Header/footer bands framing the grid, matching List/Form Browse views.
    pub(crate) header: Vec<PartView>,
    pub(crate) footer: Vec<PartView>,
    pub(crate) fields: Vec<FieldView>,
    pub(crate) records: Vec<RecordView>,
}

#[derive(Template)]
#[template(path = "view_form.html")]
pub(crate) struct FormTemplate {
    pub(crate) chrome: Chrome,
    pub(crate) table: String,
    /// The record at the flipbook's current position; `None` when empty.
    pub(crate) record: Option<FormRecord>,
}

/// One record laid out per the layout's parts/objects, with live values (#25).
pub(crate) struct FormRecord {
    pub(crate) id: i64,
    pub(crate) parts: Vec<PartView>,
}

/// A part band; objects are positioned **relative to it** (geometry contract).
/// Also the part half of the Layout-Mode read model (`/design/:layout/model`):
/// the Svelte canvas renders from the same fields the askama band macro uses, so
/// `id`/`kind` are carried for the editor's document store (#45) without changing
/// the rendered DOM.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PartView {
    pub(crate) id: i64,
    pub(crate) kind: &'static str,
    pub(crate) height: i64,
    /// The raw appearance bag (#49/Issue 7) the Band inspector edits, carried
    /// alongside the server-derived `part_style` so the inspector reads/writes the
    /// underlying `fill` key while Browse/canvas render from `part_style`. Empty
    /// string when the band has no props.
    pub(crate) props: String,
    /// Server-derived inline CSS for the band's `<div class="fm-part">` (its
    /// background fill). Interpolated identically by `_band.html` and `Band.svelte`
    /// (the #44 parity contract). Empty when the band is unstyled.
    pub(crate) part_style: String,
    pub(crate) objects: Vec<ObjectView>,
}

/// A positioned object, discriminated by `kind` (#60):
/// - `field` objects render their live `value` **only** (an input in an editable
///   view unless read-only); `field_id` names that input `f<id>`. Their caption is
///   a separate `text` object — `label` is still resolved (for the inspector) but
///   no longer rendered inline.
/// - `text` objects render their static `content`.
/// - shape objects (`shape == true`) render a styled box from `shape_style`.
/// - field/text objects render box/text styles derived from `props`.
///
/// `z` is the stacking order (CSS `z-index`); `read_only` suppresses the editable
/// input even in an editable view (per-object editability, #40/#43).
///
/// Also the object half of the Layout-Mode read model: the canvas hydrates its
/// document store from these fields. The rendered DOM (askama macro and the
/// mirroring Svelte `Band` component) uses only the visual/geometry fields, so
/// Browse and Layout stay byte-identical (#44). **Field order is the wire
/// contract** — the editor store's `renderModel` projection mirrors it key-for-key
/// (doc.svelte.ts `#toView`), so keep the two in lockstep.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ObjectView {
    pub(crate) id: i64,
    /// Portal containment (#168/#169, Model B): the owning portal's object id when
    /// this object is one of its authored columns (a child field/label positioned
    /// row-relative), else `None`. A portal enumerates its columns by matching this
    /// against its own `id`. Skipped from JSON when absent so every top-level object
    /// — and the whole parity fixture — serialises byte-identically to before (#44).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) parent_object_id: Option<i64>,
    pub(crate) kind: &'static str,
    pub(crate) field: bool,
    pub(crate) shape: bool,
    pub(crate) field_id: Option<i64>,
    pub(crate) x: i64,
    pub(crate) y: i64,
    pub(crate) w: i64,
    pub(crate) h: i64,
    pub(crate) z: i64,
    pub(crate) read_only: bool,
    pub(crate) binding: String,
    pub(crate) content: String,
    /// The raw appearance bag (#49) the Style zone edits. Carried alongside the
    /// server-derived `shape_style` so the canvas renders from `shape_style` while
    /// the inspector reads/writes the underlying `fill`/`stroke`/… keys. Empty
    /// string when the object has no props.
    pub(crate) props: String,
    pub(crate) object_style: String,
    pub(crate) text_style: String,
    pub(crate) label: String,
    pub(crate) value: String,
    /// The RAW (unformatted) field value. `value` above carries the display
    /// string (value formatting #77/#78 applied); `raw` is what an editable
    /// Browse input must commit so a formatted field is never written back as its
    /// formatted text. Skipped from the design-model JSON (the canvas renders the
    /// display `value`); the askama browse band reads it directly. Equal to
    /// `value` when no format is active.
    #[serde(skip)]
    pub(crate) raw: String,
    pub(crate) shape_style: String,
    /// Portal (#169): the terminal table's user-field names, the header row of a
    /// resolved portal. Non-empty ONLY for a portal object resolved against a base
    /// record in Browse; empty for every other object and for a portal on the
    /// design canvas (no base record). The renderer keys off this: non-empty ⇒
    /// render the related-row region (even with zero rows — an empty set renders a
    /// clean header + empty body); empty ⇒ the unresolved-frame placeholder.
    /// Skipped from JSON when empty so non-portal objects (and the design-model
    /// portal frame) serialise byte-identically to before (#44 fixture stability).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) portal_columns: Vec<String>,
    /// Portal (#170): the terminal-table field id backing each column, parallel to
    /// [`Self::portal_columns`]. A resolved portal in an editable view renders each
    /// cell as an `f<field_id>` input off these ids so a per-row commit collects the
    /// right terminal fields (the same `f<id>` contract as base-record edit). Empty
    /// for every non-portal object and for an unresolved portal frame.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) portal_field_ids: Vec<i64>,
    /// Portal (#169): one entry per related record in the resolved set, after the
    /// display-only filter (#112) and the declared sort. Each carries the terminal
    /// record id (stamped `data-related-id`) and its user-field values in column
    /// order. Empty for a non-portal object, and for a portal whose set is empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) portal_rows: Vec<PortalRowView>,
    /// Portal create-new (#171): whether this resolved portal may mint a related
    /// record — the route is create-determined (#11) AND the anchoring
    /// relationship's `allow_create` (#110) is on. The one permission on the
    /// relationship gates the affordance; the portal carries no own flag. `false`
    /// for every non-portal object, an unresolved portal frame, and a resolved
    /// portal whose route/relationship forbids create. Skipped from JSON when
    /// `false` so non-portal objects (and the design-model frame) stay byte-stable.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) portal_can_create: bool,
    /// Portal create-new (#171): the endpoint the trailing blank row posts to to
    /// mint a related record — `/browse/:layout/:base/related/:obj`. Non-empty only
    /// when [`Self::portal_can_create`] is set. Empty (and skipped from JSON) for
    /// every other object and for a portal that cannot create.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) portal_create_url: String,
}

/// One related record inside a rendered portal (#169): its terminal-table row id
/// (so an inline-edit/delete affordance can address it, #170/#172) and its user
/// field values in the portal's column order.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PortalRowView {
    pub(crate) id: i64,
    pub(crate) cells: Vec<String>,
    /// Portal inline edit (#170): the `/related/*` endpoints this row's editor
    /// posts to, precomputed server-side so the shared `.rec-edit` controller
    /// drives a child record with no client route-building. `open`/`revert`
    /// acquire and release the terminal row's lock; `action` commits it through
    /// `update_related_record`. Empty on the design canvas / non-editable render.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) open_url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) action_url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) revert_url: String,
    /// Portal delete/unlink (#172): the endpoint a per-row delete affordance posts
    /// to, removing the NEAREST related record — a to-many child is deleted, a
    /// forward to-one clears the base FK, an M:N unlinks the join row (the terminal
    /// survives; never cascades). Non-empty ONLY when the route is delete-determined
    /// AND the anchoring relationship's `allow_delete` (#110) is on — the one
    /// permission on the relationship, no portal-own flag. Empty (and skipped from
    /// JSON) otherwise, so a row that may not be deleted renders no button.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) delete_url: String,
}

/// A bindable field on the layout's primary table — the Field tool's dropdown
/// choices (#48/#62). Part of the Layout-Mode read model so the rail can offer
/// every field, not only the ones already placed.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FieldChoice {
    pub(crate) id: i64,
    pub(crate) name: String,
    /// Logical field kind (`FieldKind::as_str`) so the rail can draw type icons (#79).
    pub(crate) kind: String,
}

/// A relationship route the layout can choose for related data. These are
/// derived from declared FK constraints, not authored by portal/layout UI.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RelatedRouteChoice {
    relationship_id: i64,
    name: String,
    direction: &'static str,
    cardinality: &'static str,
    path: String,
    table_id: i64,
    table_name: String,
    from_table: i64,
    from_field: i64,
    to_table: i64,
    to_field: i64,
    /// The terminal (related) table's user fields — the column picker's choices
    /// when authoring inside a portal bound to this route (#168/#169). Same
    /// `FieldChoice` shape the primary-table Field tool offers, so the rail can
    /// retarget its picker to the related table without a second fetch.
    fields: Vec<FieldChoice>,
}

/// Project a table's fields into the Field-tool `FieldChoice` list (#48/#62) —
/// shared by the design model's primary-table fields and each related route's
/// terminal-table fields (#168/#169), so both offer the identical shape.
pub(crate) fn field_choices(fields: &[FieldMeta]) -> Vec<FieldChoice> {
    fields
        .iter()
        .map(|f| FieldChoice {
            id: f.id,
            name: f.name.clone(),
            kind: f.kind.as_str().to_string(),
        })
        .collect()
}

#[derive(Template)]
#[template(path = "view_list.html")]
pub(crate) struct ListTemplate {
    pub(crate) chrome: Chrome,
    pub(crate) table: String,
    /// Non-body parts (header/title/…) rendered once above the rows.
    pub(crate) header: Vec<PartView>,
    /// One entry per record: the Body part(s) bound to that record.
    pub(crate) rows: Vec<ListRow>,
    /// Footer/grand-summary parts rendered once below the rows.
    pub(crate) footer: Vec<PartView>,
}

/// One record's Body band(s) in List view; `current` marks the flipbook's row.
pub(crate) struct ListRow {
    id: i64,
    current: bool,
    parts: Vec<PartView>,
}

pub(crate) struct FieldView {
    pub(crate) name: String,
}

/// One Table-view column derived from a placed body field object.
pub(crate) struct TableColumn {
    pub(crate) field: FieldMeta,
    pub(crate) format: Option<serde_json::Value>,
}

pub(crate) struct RecordView {
    pub(crate) id: i64,
    pub(crate) cells: Vec<CellView>,
}

/// One Table-view cell: the field id (so editable inputs can be named `f<id>`)
/// and the current value.
pub(crate) struct CellView {
    pub(crate) field_id: i64,
    /// RAW cell value — what the editable Table input commits.
    pub(crate) value: String,
    /// Display value (value formatting #77/#78 applied). Equals `value` when the
    /// column's field object carries no `format` bag.
    pub(crate) display: String,
    /// Inline CSS for the cell input (e.g. the value-dependent negative color);
    /// empty when unstyled.
    pub(crate) style: String,
}

#[derive(Template)]
#[template(path = "design.html")]
pub(crate) struct DesignTemplate {
    pub(crate) chrome: Chrome,
    pub(crate) layout_id: i64,
    pub(crate) layout: String,
    /// Which view this layout designs (`Form`/`List`/`Table`) — shown in the
    /// status bar so the designer knows which surface they're editing (#57).
    pub(crate) view: &'static str,
}

/// The schema-builder surface (#113): a sibling to Layout Mode that manages
/// tables / fields (and, later, relationships) over the #107 `/schema/*` API.
/// App-global rather than per-layout, so it carries no current layout — the
/// Svelte island fetches the schema itself and owns the whole surface.
#[derive(Template)]
#[template(path = "schema.html")]
pub(crate) struct SchemaTemplate {
    pub(crate) chrome: Chrome,
}

/// The Layout Manager surface (#149): a sibling to Layout Mode and the schema
/// builder that lists, creates, renames, deletes, and reorders every layout
/// in the solution over the `/layouts/*` API. App-global rather than
/// per-layout, so it carries no current layout — the Svelte island fetches
/// the list itself and owns the whole surface.
#[derive(Template)]
#[template(path = "layouts.html")]
pub(crate) struct LayoutsTemplate {
    pub(crate) chrome: Chrome,
}

/// Resolve a field object's binding to its (field, field_id, label, value) for the
/// current record. Interim two-segment resolver: the last dot-path segment is the
/// field name, matched case-insensitively against `by_name` (lowercased field name
/// → `(display name, value)`). The full relationship resolver replaces this (#11).
///
/// Non-field objects (text / shapes) resolve to no live value — text renders from
/// its own `content` slot and shapes from `props`, neither of which is
/// record-dependent (#60). Only the bound value/label come from the record here.
fn resolve_object(
    o: &ObjectMeta,
    by_name: &HashMap<String, (i64, String, String, FieldKind)>,
) -> (bool, Option<i64>, String, String, Option<FieldKind>) {
    match (o.kind, o.binding.as_deref()) {
        (ObjectKind::Field, Some(binding)) => {
            let seg = binding.rsplit('.').next().unwrap_or(binding).to_lowercase();
            match by_name.get(&seg) {
                Some((id, label, value, kind)) => {
                    (true, Some(*id), label.clone(), value.clone(), Some(*kind))
                }
                // A binding that doesn't resolve yet (e.g. a relationship path)
                // still renders a useful placeholder instead of a blank object.
                None => (true, None, binding.to_string(), binding.to_string(), None),
            }
        }
        _ => (false, None, String::new(), String::new(), None),
    }
}

/// A record's field values keyed by lowercased field name → (field id, display
/// name, value) — the lookup `resolve_object` binds against.
pub(crate) fn by_name_map(
    fields: &[FieldMeta],
    cells: Vec<String>,
) -> HashMap<String, (i64, String, String, FieldKind)> {
    fields
        .iter()
        .zip(cells)
        .map(|(f, value)| (f.name.to_lowercase(), (f.id, f.name.clone(), value, f.kind)))
        .collect()
}

pub(crate) fn by_name_for_rec(
    sol: &Solution,
    table: &TableMeta,
    fields: &[FieldMeta],
    rec: Option<i64>,
) -> HashMap<String, (i64, String, String, FieldKind)> {
    // COUNT + LIMIT/OFFSET instead of materialising the whole found set: this
    // runs on every small object mutation, which only needs the one record.
    let total = sol.record_count(table).unwrap();
    let rec = clamp_rec_n(rec, total);
    if rec < 1 {
        return HashMap::new();
    }
    let Some(id) = sol.record_id_at(table, rec).unwrap() else {
        return HashMap::new();
    };
    match sol.get_record(table, fields, id).unwrap() {
        Some(cells) => by_name_map(fields, cells),
        None => HashMap::new(),
    }
}

/// The record-independent render state of one object: its derived CSS strings,
/// resolved content slot, and pre-parsed `format` bag. Computing these is pure
/// projection of the object's own metadata, so per-record renders (every List
/// row repeats the same body objects) reuse one of these instead of re-deriving
/// styles and re-parsing props per record.
pub(crate) struct PreparedObject {
    meta: ObjectMeta,
    shape: bool,
    /// The text slot is only meaningful for `text` objects; fields/shapes carry
    /// none, so the renderer never reads a stray content.
    content: String,
    object_style: String,
    text_style: String,
    shape_style: String,
    /// The `format` sub-bag of the object's props (#77/#78), pre-parsed.
    format: Option<serde_json::Value>,
}

/// Precompute an object's record-independent render state (see [`PreparedObject`]).
pub(crate) fn prepare_object(o: ObjectMeta) -> PreparedObject {
    let shape = o.kind.is_shape();
    let content = match o.kind {
        ObjectKind::Text => o.content.clone().unwrap_or_default(),
        _ => String::new(),
    };
    let shape_style = if shape {
        shape_style(o.kind, o.props.as_deref())
    } else {
        String::new()
    };
    let object_style = object_style(o.kind, o.props.as_deref());
    let text_style = text_style(o.kind, o.props.as_deref());
    let format = parse_props(o.props.as_deref()).and_then(|v| v.get("format").cloned());
    PreparedObject {
        meta: o,
        shape,
        content,
        object_style,
        text_style,
        shape_style,
        format,
    }
}

/// The anchor a portal object renders against (#169): the live solution plus the
/// base record its route is rooted at. Threaded ONLY through the Browse Form/List
/// render paths — a header/footer band, the design canvas, and the create/restore
/// handlers pass `None`, so a portal there renders its unresolved frame rather
/// than issuing a related read with no base record.
pub(crate) struct PortalCtx<'a> {
    pub(crate) sol: &'a Solution,
    /// The layout the portal lives on — used to address the row's `/related/*`
    /// edit endpoints (#170), which are scoped under `/browse/:layout/…`.
    pub(crate) layout_id: i64,
    pub(crate) base_table: i64,
    pub(crate) base_id: i64,
}

/// The resolved render state of a portal object (#169/#170/#171): the terminal
/// table's column names, the backing field ids (parallel to the columns, so an
/// editable row emits `f<id>` inputs), the related rows after the #112 filter +
/// declared sort, and the create-new gate. Default (all-empty / `can_create`
/// false) is the unresolved frame.
#[derive(Default)]
struct PortalResolved {
    columns: Vec<String>,
    field_ids: Vec<i64>,
    rows: Vec<PortalRowView>,
    /// #171: the route is create-determined AND the anchor relationship's
    /// `allow_create` is on — the trailing blank create row may render.
    can_create: bool,
    /// #171: where that blank row posts to mint a related record. Non-empty only
    /// when `can_create`.
    create_url: String,
}

/// Resolve a portal object's bound route against `ctx` into its [`PortalResolved`]
/// (#169/#170/#171). `columns` are the terminal table's user-field names;
/// `field_ids` are the backing field ids, parallel to the columns (so an editable
/// row can emit `f<id>` inputs, #170); `rows` are the related records after the
/// display-only filter (#112) and the declared sort, each carrying its inline-edit
/// endpoint URLs; `can_create`/`create_url` gate the trailing blank create row
/// (#171).
///
/// A blank/unresolvable binding yields the default (all-empty) so the frame renders
/// its unresolved-placeholder branch; a route that resolves to zero related records
/// yields the columns + an empty body so the header renders over a clean empty body.
fn resolve_portal(o: &ObjectMeta, ctx: &PortalCtx) -> PortalResolved {
    let Some(binding) = o.binding.as_deref().filter(|b| !b.is_empty()) else {
        return PortalResolved::default();
    };
    let Ok(route) = ctx.sol.resolve_path(ctx.base_table, binding) else {
        return PortalResolved::default();
    };
    let Ok(fields) = ctx.sol.fields(route.terminal_table) else {
        return PortalResolved::default();
    };
    let filter = parse_portal_filter(o.props.as_deref());
    let mut records = ctx
        .sol
        .read_related_records_filtered(&route, ctx.base_id, &filter)
        .unwrap_or_default();
    apply_portal_sort(o.props.as_deref(), &fields, &mut records);
    let columns = fields.iter().map(|f| f.name.clone()).collect();
    let field_ids = fields.iter().map(|f| f.id).collect();
    // The anchoring relationship (the route's first hop) carries the referential
    // flags (#110) that gate both the create-new and the delete/unlink affordances.
    let anchor_rel = route
        .hops
        .first()
        .and_then(|h| ctx.sol.relationship_by_id(h.relationship_id).ok().flatten());
    // Delete/unlink gate (#172): the route must resolve to a determined nearest
    // record (DirectFk or join-table M:N — exactly the classes
    // `delete_related_record` supports) AND the anchoring relationship's
    // `allow_delete` (#110) must be on. One permission on the relationship — the
    // portal has no own flag. Suppressed on an Undetermined/base route or when
    // `allow_delete` is off; the engine's `delete_related_record` enforces the same
    // gate again.
    let can_delete = route.class.create_determined()
        && anchor_rel.as_ref().is_some_and(|r| r.allow_delete);
    // Per-row inline-edit endpoints (#170), scoped to this portal object and the
    // terminal row id: `/browse/:layout/:base/related/:obj/:rec[/open|/revert|/delete]`.
    let base = format!(
        "/browse/{}/{}/related/{}",
        ctx.layout_id, ctx.base_id, o.id
    );
    let rows = records
        .into_iter()
        .map(|r| PortalRowView {
            open_url: format!("{base}/{}/open", r.id),
            action_url: format!("{base}/{}", r.id),
            revert_url: format!("{base}/{}/revert", r.id),
            delete_url: if can_delete {
                format!("{base}/{}/delete", r.id)
            } else {
                String::new()
            },
            id: r.id,
            cells: r.cells,
        })
        .collect();
    // Create-new gate (#171): the route must be create-determined (#11) AND the
    // anchoring relationship (the first hop) must permit create (#110). One
    // permission on the relationship — the portal has no own flag. The `/new`
    // affordance is suppressed on an Undetermined route or when `allow_create` is
    // off; the engine's `create_related_record` enforces the same gate again.
    let can_create =
        route.class.create_determined() && anchor_rel.as_ref().is_some_and(|r| r.allow_create);
    let create_url = if can_create {
        format!("/browse/{}/{}/related/{}", ctx.layout_id, ctx.base_id, o.id)
    } else {
        String::new()
    };
    PortalResolved {
        columns,
        field_ids,
        rows,
        can_create,
        create_url,
    }
}

/// Parse a portal's optional display-only read filter (#112) from its `props`
/// JSON: `{"filter":{"clauses":[{"field":<id>,"op":"eq|ne|lt|le|gt|ge",
/// "value":"…"|"parentField":<id>}, …]}}`. Absent/malformed ⇒ no refinement. The
/// engine validates the terminal-field ids, so a stray id surfaces as an empty
/// read (display-only), never a panic.
fn parse_portal_filter(props: Option<&str>) -> RelatedFilter {
    let Some(v) = parse_props(props) else {
        return RelatedFilter::none();
    };
    let Some(clauses) = v
        .get("filter")
        .and_then(|f| f.get("clauses"))
        .and_then(|c| c.as_array())
    else {
        return RelatedFilter::none();
    };
    let clauses = clauses
        .iter()
        .filter_map(|c| {
            let field_id = c.get("field").and_then(serde_json::Value::as_i64)?;
            let op = match c.get("op").and_then(serde_json::Value::as_str).unwrap_or("eq") {
                "ne" => FilterOp::Ne,
                "lt" => FilterOp::Lt,
                "le" => FilterOp::Le,
                "gt" => FilterOp::Gt,
                "ge" => FilterOp::Ge,
                _ => FilterOp::Eq,
            };
            let rhs = match c.get("parentField").and_then(serde_json::Value::as_i64) {
                Some(pf) => FilterOperand::ParentField(pf),
                None => FilterOperand::Value(
                    c.get("value")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                ),
            };
            Some(FilterClause { field_id, op, rhs })
        })
        .collect();
    RelatedFilter { clauses }
}

/// Apply a portal's optional declared sort from its `props` JSON:
/// `{"sort":{"field":<id>,"dir":"asc"|"desc"}}`. Numeric-aware (both cells parse
/// as `f64` ⇒ numeric order, else byte-wise), stable, and a no-op when absent or
/// the field isn't a column. Ordering is done here rather than in-engine because
/// the read set is defined by FK membership; sort is a presentation choice.
fn apply_portal_sort(props: Option<&str>, fields: &[FieldMeta], records: &mut [Record]) {
    let Some(v) = parse_props(props) else { return };
    let Some(sort) = v.get("sort") else { return };
    let Some(field_id) = sort.get("field").and_then(serde_json::Value::as_i64) else {
        return;
    };
    let Some(idx) = fields.iter().position(|f| f.id == field_id) else {
        return;
    };
    let desc = sort.get("dir").and_then(serde_json::Value::as_str) == Some("desc");
    records.sort_by(|a, b| {
        let av = a.cells.get(idx).map(String::as_str).unwrap_or("");
        let bv = b.cells.get(idx).map(String::as_str).unwrap_or("");
        let ord = match (av.parse::<f64>(), bv.parse::<f64>()) {
            (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
            _ => av.cmp(bv),
        };
        if desc { ord.reverse() } else { ord }
    });
}

/// Project a prepared object against one record's `by_name` map — the
/// record-dependent half of [`object_view`]. `portal` supplies the anchor a
/// portal object resolves its related rows against (#169); `None` renders a
/// portal as its unresolved frame (design canvas, header/footer, create/restore).
fn prepared_object_view(
    p: &PreparedObject,
    by_name: &HashMap<String, (i64, String, String, FieldKind)>,
    portal: Option<&PortalCtx>,
) -> ObjectView {
    let o = &p.meta;
    let portal_resolved = match (o.kind.is_portal(), portal) {
        (true, Some(ctx)) => resolve_portal(o, ctx),
        _ => PortalResolved::default(),
    };
    let PortalResolved {
        columns: portal_columns,
        field_ids: portal_field_ids,
        rows: portal_rows,
        can_create: portal_can_create,
        create_url: portal_create_url,
    } = portal_resolved;
    let (field, field_id, label, raw_value, field_kind) = resolve_object(o, by_name);
    let mut text_style = p.text_style.clone();
    // Value formatting (#77/#78) is display-only: applied to the resolved value
    // for BOTH Browse and the design canvas, driven by the `format` sub-bag of
    // the object's props and the bound field's kind. A negative-number color is
    // value-dependent, so it rides `text_style` here (appended last, so it wins
    // over any static textColor) rather than the static props CSS. An unresolved
    // binding (`field_kind == None`) leaves the placeholder untouched.
    let value = match field_kind {
        Some(kind) => {
            let formatted = format::format_value(&raw_value, p.format.as_ref(), kind);
            if let Some(color) = formatted.color {
                text_style.push_str(&format!("color:{color};"));
            }
            formatted.text
        }
        None => raw_value.clone(),
    };
    ObjectView {
        id: o.id,
        parent_object_id: o.parent_object_id,
        kind: o.kind.as_str(),
        field,
        shape: p.shape,
        field_id,
        x: o.x,
        y: o.y,
        w: o.w,
        h: o.h,
        z: o.z,
        read_only: o.read_only,
        binding: o.binding.clone().unwrap_or_default(),
        content: p.content.clone(),
        props: o.props.clone().unwrap_or_default(),
        object_style: p.object_style.clone(),
        text_style,
        label,
        value,
        raw: raw_value,
        shape_style: p.shape_style.clone(),
        portal_columns,
        portal_field_ids,
        portal_rows,
        portal_can_create,
        portal_create_url,
    }
}

/// Resolve one object into its `ObjectView` (#44/#60), bound against `by_name`.
/// The single per-object projection shared by [`render_part`] and the create
/// handler, so an object placed on the canvas serialises byte-identically to one
/// read back from the model — there is no second mapping to drift.
pub(crate) fn object_view(
    o: &ObjectMeta,
    by_name: &HashMap<String, (i64, String, String, FieldKind)>,
) -> ObjectView {
    prepared_object_view(&prepare_object(o.clone()), by_name, None)
}

pub(crate) fn object_view_for_rec(
    sol: &Solution,
    layout_id: i64,
    object_id: i64,
    rec: Option<i64>,
) -> Option<ObjectView> {
    let (_lay, table) = layout_table(sol, layout_id)?;
    let fields = sol.fields(table.id).ok()?;
    let by_name = by_name_for_rec(sol, &table, &fields, rec);
    let object = sol.object_by_id(layout_id, object_id).ok()??;
    Some(object_view(&object, &by_name))
}

/// Shared tail of the single-object mutation handlers (binding / binding-path /
/// content / read-only): 404 when the write matched no row, otherwise re-project
/// the object against `rec` exactly as a model fetch would.
pub(crate) fn updated_object_view(
    sol: &Solution,
    layout_id: i64,
    object_id: i64,
    rec: Option<i64>,
    updated: usize,
) -> AppResult<Json<ObjectView>> {
    if updated == 0 {
        return Err(AppError::not_found());
    }
    object_view_for_rec(sol, layout_id, object_id, rec)
        .map(Json)
        .ok_or_else(AppError::not_found)
}

pub(crate) fn related_route_choices(sol: &Solution, table: &TableMeta) -> Vec<RelatedRouteChoice> {
    sol.relationships()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|rel| {
            if rel.from_table == table.id {
                let target = sol.table_by_id(rel.to_table).ok().flatten()?;
                let fields = field_choices(&sol.fields(target.id).unwrap_or_default());
                Some(RelatedRouteChoice {
                    relationship_id: rel.id,
                    name: rel.name.clone(),
                    direction: "forward",
                    cardinality: "toOne",
                    path: rel.name.clone(),
                    table_id: target.id,
                    table_name: target.name,
                    from_table: rel.from_table,
                    from_field: rel.from_field,
                    to_table: rel.to_table,
                    to_field: rel.to_field,
                    fields,
                })
            } else if rel.to_table == table.id {
                let target = sol.table_by_id(rel.from_table).ok().flatten()?;
                let fields = field_choices(&sol.fields(target.id).unwrap_or_default());
                Some(RelatedRouteChoice {
                    relationship_id: rel.id,
                    name: rel.name.clone(),
                    direction: "reverse",
                    cardinality: "toMany",
                    path: rel.name.clone(),
                    table_id: target.id,
                    table_name: target.name,
                    from_table: rel.from_table,
                    from_field: rel.from_field,
                    to_table: rel.to_table,
                    to_field: rel.to_field,
                    fields,
                })
            } else {
                None
            }
        })
        .collect()
}

/// All of a layout's parts with their objects, fetched once (position order,
/// objects in stacking order). The shared prefetch for handlers that would
/// otherwise re-query the same parts/objects several times per request.
pub(crate) fn layout_parts_with_objects(
    sol: &Solution,
    layout_id: i64,
) -> Vec<(PartMeta, Vec<ObjectMeta>)> {
    sol.parts(layout_id)
        .unwrap()
        .into_iter()
        .map(|p| {
            let objects = sol.objects(p.id).unwrap();
            (p, objects)
        })
        .collect()
}

/// Render one part from prefetched objects — [`render_part`]'s core, for
/// callers that already hold the layout's parts+objects.
pub(crate) fn render_part_with_objects(
    part: &PartMeta,
    objects: &[ObjectMeta],
    by_name: &HashMap<String, (i64, String, String, FieldKind)>,
    portal: Option<&PortalCtx>,
) -> PartView {
    PartView {
        id: part.id,
        kind: part.kind.as_str(),
        height: part.height,
        props: part.props.clone().unwrap_or_default(),
        part_style: part_style(part.props.as_deref()),
        objects: objects
            .iter()
            .map(|o| prepared_object_view(&prepare_object(o.clone()), by_name, portal))
            .collect(),
    }
}

/// Render one part's objects, positioned and bound against `by_name` (an empty
/// map leaves field values blank — used for header/footer with no record).
/// `portal` supplies a portal object's anchor (#169); `None` renders portals as
/// unresolved frames.
pub(crate) fn render_part(
    sol: &Solution,
    part: &PartMeta,
    by_name: &HashMap<String, (i64, String, String, FieldKind)>,
    portal: Option<&PortalCtx>,
) -> PartView {
    render_part_with_objects(part, &sol.objects(part.id).unwrap(), by_name, portal)
}

/// A part with its objects' record-independent render state precomputed, so a
/// per-record render (List view repeats the body band for every record) only
/// re-resolves the bound values instead of re-querying objects and re-deriving
/// their CSS each time.
pub(crate) struct PreparedPart {
    id: i64,
    kind: &'static str,
    height: i64,
    props: String,
    part_style: String,
    objects: Vec<PreparedObject>,
}

/// Precompute a part's record-independent render state from prefetched objects.
pub(crate) fn prepare_part(part: &PartMeta, objects: Vec<ObjectMeta>) -> PreparedPart {
    PreparedPart {
        id: part.id,
        kind: part.kind.as_str(),
        height: part.height,
        props: part.props.clone().unwrap_or_default(),
        part_style: part_style(part.props.as_deref()),
        objects: objects.into_iter().map(prepare_object).collect(),
    }
}

/// Render a prepared part against one record's `by_name` map. Emits the same
/// `PartView` as [`render_part`], minus the per-record re-derivation.
pub(crate) fn render_prepared_part(
    prep: &PreparedPart,
    by_name: &HashMap<String, (i64, String, String, FieldKind)>,
    portal: Option<&PortalCtx>,
) -> PartView {
    PartView {
        id: prep.id,
        kind: prep.kind,
        height: prep.height,
        props: prep.props.clone(),
        part_style: prep.part_style.clone(),
        objects: prep
            .objects
            .iter()
            .map(|p| prepared_object_view(p, by_name, portal))
            .collect(),
    }
}

/// Project a part into the objects-free `PartView` the part-mutation handlers
/// echo (create / height / kind / props) — one literal instead of four.
pub(crate) fn part_view(part: &PartMeta) -> PartView {
    PartView {
        id: part.id,
        kind: part.kind.as_str(),
        height: part.height,
        props: part.props.clone().unwrap_or_default(),
        part_style: part_style(part.props.as_deref()),
        objects: Vec::new(),
    }
}

/// Shared tail of the part-mutation handlers (height / kind / props): 404 when
/// the write matched no row, otherwise re-read the part and echo its view.
pub(crate) fn updated_part_view(
    sol: &Solution,
    layout_id: i64,
    part_id: i64,
    updated: usize,
) -> AppResult<Json<PartView>> {
    if updated == 0 {
        return Err(AppError::not_found());
    }
    let part = sol
        .part_by_id(layout_id, part_id)
        .unwrap()
        .ok_or_else(AppError::not_found)?;
    Ok(Json(part_view(&part)))
}

/// Canvas width for a layout, from its prefetched parts+objects
/// ([`layout_parts_with_objects`]): the rightmost object edge + a margin.
/// Geometry is record-independent, so this is the same for every record.
pub(crate) fn canvas_width(parts: &[(PartMeta, Vec<ObjectMeta>)]) -> i64 {
    let mut w = 0i64;
    for (_p, objects) in parts {
        for o in objects {
            w = w.max(o.x + o.w);
        }
    }
    w + 24
}

/// Build the Form-view render of the record at flipbook position `rec`: the
/// layout's parts, each with its objects positioned and bound to live values.
/// `None` when the found set is empty (`rec == 0`) or the row vanished.
pub(crate) fn build_form_record(
    sol: &Solution,
    layout_id: i64,
    table: &TableMeta,
    fields: &[FieldMeta],
    ids: &[i64],
    rec: i64,
) -> Option<FormRecord> {
    if rec <= 0 {
        return None;
    }
    let id = ids[(rec - 1) as usize];
    let cells = sol.get_record(table, fields, id).unwrap()?;
    let by_name = by_name_map(fields, cells);
    // Portals in the Form resolve their related rows against THIS record (#169).
    let portal = PortalCtx {
        sol,
        layout_id,
        base_table: table.id,
        base_id: id,
    };
    let parts = sol
        .parts(layout_id)
        .unwrap()
        .iter()
        .map(|p| render_part(sol, p, &by_name, Some(&portal)))
        .collect();
    Some(FormRecord { id, parts })
}

/// The header and footer bands of a layout, rendered once with no record bound,
/// from prefetched parts+objects ([`layout_parts_with_objects`]). Shared by List
/// and Table Browse views so both frame their rows with the same bands: header /
/// sub-summary render above, footer / grand-summary below.
pub(crate) fn build_bands(
    parts: &[(PartMeta, Vec<ObjectMeta>)],
) -> (Vec<PartView>, Vec<PartView>) {
    let no_record = HashMap::new();
    let (mut header, mut footer) = (Vec::new(), Vec::new());
    for (p, objects) in parts {
        match p.kind {
            PartKind::Footer | PartKind::GrandSummary => {
                footer.push(render_part_with_objects(p, objects, &no_record, None))
            }
            PartKind::Header | PartKind::SubSummary => {
                header.push(render_part_with_objects(p, objects, &no_record, None))
            }
            PartKind::Body => {}
        }
    }
    (header, footer)
}

/// Build the List-view render: header/footer parts once, the Body part(s)
/// repeated per record bound to its values. `current_rec` (1-based) marks the
/// flipbook's row. Returns `(header, rows, footer)`. Parts+objects are fetched
/// once and the body bands' record-independent state is precomputed, so the
/// per-record loop only resolves values (one bulk record fetch, no N+1).
pub(crate) fn build_list(
    sol: &Solution,
    layout_id: i64,
    table: &TableMeta,
    fields: &[FieldMeta],
    current_rec: i64,
) -> (Vec<PartView>, Vec<ListRow>, Vec<PartView>) {
    let parts = layout_parts_with_objects(sol, layout_id);
    let (header, footer) = build_bands(&parts);
    let body_parts: Vec<PreparedPart> = parts
        .into_iter()
        .filter(|(p, _)| p.kind == PartKind::Body)
        .map(|(p, objects)| prepare_part(&p, objects))
        .collect();

    let mut rows = Vec::new();
    for (i, r) in sol.list_records(table, fields).unwrap().into_iter().enumerate() {
        let base_id = r.id;
        let by_name = by_name_map(fields, r.cells);
        // Each row's portals anchor on that row's record (#169).
        let portal = PortalCtx {
            sol,
            layout_id,
            base_table: table.id,
            base_id,
        };
        let parts = body_parts
            .iter()
            .map(|p| render_prepared_part(p, &by_name, Some(&portal)))
            .collect();
        rows.push(ListRow {
            id: base_id,
            current: (i as i64) + 1 == current_rec,
            parts,
        });
    }
    (header, rows, footer)
}

/// Project Table Browse columns from field objects placed in Body parts. Schema
/// fields that are not placed on the Table layout are intentionally omitted; the
/// layout body is the source of truth for Table Browse's grid. Duplicate bindings
/// collapse to the first object in visual column order.
pub(crate) fn table_body_columns(
    parts: &[(PartMeta, Vec<ObjectMeta>)],
    fields: &[FieldMeta],
) -> Vec<TableColumn> {
    let by_name: HashMap<String, usize> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name.to_lowercase(), i))
        .collect();
    let mut candidates = Vec::new();
    for (p, objects) in parts {
        if p.kind != PartKind::Body {
            continue;
        }
        for o in objects {
            if o.kind != ObjectKind::Field {
                continue;
            }
            let Some(binding) = o.binding.as_deref() else {
                continue;
            };
            let seg = binding.rsplit('.').next().unwrap_or(binding).to_lowercase();
            let Some(&idx) = by_name.get(&seg) else {
                continue;
            };
            let format = parse_props(o.props.as_deref()).and_then(|v| v.get("format").cloned());
            candidates.push((
                o.x,
                o.y,
                o.z,
                o.id,
                TableColumn {
                    field: fields[idx].clone(),
                    format,
                },
            ));
        }
    }
    candidates.sort_by_key(|(x, y, z, id, _)| (*x, *y, *z, *id));

    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter_map(|(_, _, _, _, column)| {
            if seen.insert(column.field.id) {
                Some(column)
            } else {
                None
            }
        })
        .collect()
}
