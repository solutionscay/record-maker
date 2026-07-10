//! The Layout-Mode design API (#44/#48): the read model (`design_model`)
//! plus the object/part/group mutation handlers the Svelte editor commits
//! through.

use std::collections::{BTreeMap, HashMap};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use record_maker_engine::{
    FieldKind, NewObject, ObjectGroup, ObjectKind, PartKind, RestoreObject, RestoreResult, Solution,
};

use crate::style::{object_style, shape_style, text_style};
use crate::viewmodel::{
    by_name_for_rec, by_name_map, canvas_width, clamp_rec, field_choices, layout_parts_with_objects,
    layout_table, object_view, part_view, related_route_choices, render_part_with_objects,
    updated_object_view, updated_part_view, FieldChoice, ObjectView, PartView, RelatedRouteChoice,
};
use crate::{not_found, AppError, AppResult, AppState};

/// The Layout-Mode read model (#44): the layout's parts/objects with resolved
/// labels + live values for record `?rec=N` (1-based; defaults to the first
/// record, blank values when the table is empty — geometry is record-independent,
/// so an empty table still has a designable canvas). The Svelte canvas renders
/// from this over the same axum contract Browse uses (ADR #42).
/// `render_part_with_objects` is the single server-side resolver shared with
/// Browse, so values/bindings can never diverge between the two surfaces; only
/// the DOM emission is mirrored client-side (and guarded by a parity test).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DesignModel {
    layout_id: i64,
    rec: i64,
    total: i64,
    width: i64,
    /// The layout's Browse view (`form` | `list` | `table`) — the client gates the
    /// summary part-kinds on it (a form allows only header/body/footer, Issue 3).
    view: String,
    /// The primary table's fields — what the Create zone's Field tool offers
    /// (#48/#62). Geometry-independent, so the same list rides every record.
    fields: Vec<FieldChoice>,
    /// Constraint-derived related routes from this layout's base table. Portal
    /// authoring selects from this list rather than creating relationships inline.
    related_routes: Vec<RelatedRouteChoice>,
    parts: Vec<PartView>,
    /// Durable object groups (#75). Objects remain rendered under their parts;
    /// these ids only drive Layout-mode selection/move behaviour.
    groups: Vec<ObjectGroupView>,
    /// Per-object-kind capability records, keyed by kind string — the engine's
    /// single capability table ([`ObjectKind::capabilities`]), so the editor's
    /// "can this kind be filled / text-formatted / bound?" gates read one
    /// definition instead of transcribing it.
    capabilities: BTreeMap<&'static str, ObjectCapabilitiesView>,
}

/// One [`record_maker_engine::ObjectCapabilities`] record on the wire.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ObjectCapabilitiesView {
    fill: bool,
    stroke: bool,
    text_format: bool,
    content_slot: bool,
    bindable: bool,
}

/// The full per-kind capability table for the design model, keyed by the kind's
/// wire string (`field`/`text`/`rect`/`line`/`ellipse`).
fn kind_capabilities() -> BTreeMap<&'static str, ObjectCapabilitiesView> {
    ObjectKind::ALL
        .iter()
        .map(|k| {
            let c = k.capabilities();
            (
                k.as_str(),
                ObjectCapabilitiesView {
                    fill: c.fill,
                    stroke: c.stroke,
                    text_format: c.text_format,
                    content_slot: c.content_slot,
                    bindable: c.bindable,
                },
            )
        })
        .collect()
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ObjectGroupView {
    id: i64,
    object_ids: Vec<i64>,
}

fn object_group_view(g: ObjectGroup) -> ObjectGroupView {
    ObjectGroupView {
        id: g.id,
        object_ids: g.object_ids,
    }
}

pub(crate) async fn design_model(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Query(q): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some((lay, table)) = layout_table(&sol, layout_id) else {
        return not_found("layout", layout_id);
    };
    let ids = sol.record_ids(&table).unwrap();
    let total = ids.len() as i64;
    let rec = clamp_rec(&q, total);
    // `all_fields`: the Field tool's choices and any already-placed object's live
    // value must both see the system primary key (#156), which `fields()` excludes.
    let fields = sol.all_fields(table.id).unwrap();
    // Bind to the record at `rec` when present; otherwise render geometry blank.
    let by_name = if rec >= 1 {
        match sol
            .get_record(&table, &fields, ids[(rec - 1) as usize])
            .unwrap()
        {
            Some(cells) => by_name_map(&fields, cells),
            None => HashMap::new(),
        }
    } else {
        HashMap::new()
    };
    // One parts+objects fetch feeds both the rendered parts and the width.
    let parts_objects = layout_parts_with_objects(&sol, layout_id);
    let width = canvas_width(&parts_objects);
    let parts = parts_objects
        .iter()
        .map(|(p, objects)| render_part_with_objects(p, objects, &by_name, None))
        .collect();
    let model = DesignModel {
        layout_id,
        rec,
        total,
        width,
        view: lay.view.clone(),
        fields: field_choices(&fields),
        related_routes: related_route_choices(&sol, &table),
        parts,
        groups: sol
            .object_groups(layout_id)
            .unwrap()
            .into_iter()
            .map(object_group_view)
            .collect(),
        capabilities: kind_capabilities(),
    };
    axum::Json(model).into_response()
}

/// One value + format spec for the inspector's "Sample" preview (#77/#78).
#[derive(serde::Deserialize)]
pub(crate) struct FormatSampleBody {
    raw: String,
    kind: String,
    format: Option<serde_json::Value>,
}

/// The rendered sample: display text plus an optional colour override (e.g. a
/// negative number's `negativeColor`). Mirrors [`crate::format::Formatted`].
#[derive(serde::Serialize)]
pub(crate) struct FormatSampleView {
    text: String,
    color: Option<String>,
}

/// `POST /design/format-sample` — render one raw value through the REAL
/// formatter (`crates/server/src/format.rs`) for the inspector's live Sample
/// line, so the format rules exist exactly once (no client-side mirror to keep
/// in step). Stateless: pure function of the request body.
pub(crate) async fn format_sample(
    Json(body): Json<FormatSampleBody>,
) -> AppResult<Json<FormatSampleView>> {
    let kind = FieldKind::parse(&body.kind)
        .ok_or_else(|| AppError::bad_request(format!("unknown field kind: {}", body.kind)))?;
    let f = crate::format::format_value(&body.raw, body.format.as_ref(), kind);
    Ok(Json(FormatSampleView {
        text: f.text,
        color: f.color,
    }))
}

/// The geometry a Layout-canvas drag/resize commits for one object (#15) —
/// part-relative px integers mirroring the #43 geometry contract.
#[derive(serde::Deserialize)]
pub(crate) struct GeometryUpdate {
    x: i64,
    y: i64,
    w: i64,
    h: i64,
}

/// Persist one object's new geometry from the Layout canvas (#15): the canvas
/// POSTs `{x,y,w,h}` after a drag and this writes it to `meta_object`, scoped to
/// the layout. Coordinates clamp to the canvas origin (no negative part-relative
/// geometry) and to a 1px minimum size, so a stray value can't push an object off
/// the top-left or collapse it. 200 on success; 404 when no such object belongs to
/// the layout. The geometry is authoritative, so Browse shows it on the next read.
pub(crate) async fn update_object_geometry(
    State(st): State<AppState>,
    Path((layout_id, object_id)): Path<(i64, i64)>,
    Json(geom): Json<GeometryUpdate>,
) -> AppResult<StatusCode> {
    let sol = st.sol.lock().unwrap();
    let updated = sol
        .set_object_geometry(
            layout_id,
            object_id,
            geom.x.max(0),
            geom.y.max(0),
            geom.w.max(1),
            geom.h.max(1),
        )
        .unwrap();
    if updated == 0 {
        return Err(AppError::not_found());
    }
    Ok(StatusCode::OK)
}

/// A cross-band move from the Layout canvas (#46): the object's new owning part
/// and its part-relative origin. `x`/`y` clamp to the canvas origin server-side.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ObjectPartUpdate {
    part_id: i64,
    x: i64,
    y: i64,
}

/// Persist an object's new band membership from the Layout canvas (#46): a drag
/// that crosses a band boundary POSTs `{partId,x,y}` and this reparents the object
/// to that part, scoped to the layout and clamped to the canvas origin like the
/// geometry commit. 200 on success; 404 when the object or target part isn't in
/// the layout. Authoritative, so Browse reflects the new band on the next read.
pub(crate) async fn update_object_part(
    State(st): State<AppState>,
    Path((layout_id, object_id)): Path<(i64, i64)>,
    Json(body): Json<ObjectPartUpdate>,
) -> AppResult<StatusCode> {
    let sol = st.sol.lock().unwrap();
    let updated = sol
        .set_object_part(
            layout_id,
            object_id,
            body.part_id,
            body.x.max(0),
            body.y.max(0),
        )
        .unwrap();
    if updated == 0 {
        return Err(AppError::not_found());
    }
    Ok(StatusCode::OK)
}

/// One object's geometry in a bulk commit (#46): the object id plus its new box.
#[derive(serde::Deserialize)]
pub(crate) struct ObjectGeometry {
    id: i64,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
}

/// Persist a whole group's geometry from the Layout canvas (#46): the canvas
/// POSTs `[{id,x,y,w,h}, …]` after a multi-select drag/resize and this writes
/// them in one transaction, each scoped to the layout and clamped like the
/// single-object commit. Always 200 (unknown ids are simply skipped); the body
/// is the count actually updated, so the client can detect a stale selection.
pub(crate) async fn update_objects_geometry(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Json(items): Json<Vec<ObjectGeometry>>,
) -> impl IntoResponse {
    let clamped: Vec<(i64, i64, i64, i64, i64)> = items
        .iter()
        .map(|g| (g.id, g.x.max(0), g.y.max(0), g.w.max(1), g.h.max(1)))
        .collect();
    let mut sol = st.sol.lock().unwrap();
    let updated = sol.set_objects_geometry(layout_id, &clamped).unwrap();
    (StatusCode::OK, updated.to_string()).into_response()
}

/// One object's stacking order in a bulk commit (#83): the object id plus its new `z`.
#[derive(serde::Deserialize)]
pub(crate) struct ObjectZ {
    id: i64,
    z: i64,
}

/// Persist a group's stacking order from the Arrange panel (#83): the panel
/// re-densifies a part's `z` after a Bring-to-Front / Send-to-Back / step command
/// and POSTs `[{id,z}, …]`; this writes them in one transaction, each scoped to
/// the layout. Always 200 (unknown ids are simply skipped); the body is the count
/// actually updated, mirroring [`update_objects_geometry`].
pub(crate) async fn update_objects_z(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Json(items): Json<Vec<ObjectZ>>,
) -> impl IntoResponse {
    let pairs: Vec<(i64, i64)> = items.iter().map(|z| (z.id, z.z)).collect();
    let mut sol = st.sol.lock().unwrap();
    let updated = sol.set_objects_z(layout_id, &pairs).unwrap();
    (StatusCode::OK, updated.to_string()).into_response()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateObjectGroupBody {
    id: Option<i64>,
    object_ids: Vec<i64>,
}

/// Create a durable group over selected layout objects (#75). This is a metadata
/// relationship only: no child geometry/style/z values change. Re-grouping
/// objects already in groups replaces their old memberships.
pub(crate) async fn create_object_group(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Json(body): Json<CreateObjectGroupBody>,
) -> AppResult<Json<ObjectGroupView>> {
    let mut sol = st.sol.lock().unwrap();
    let group = sol
        .create_object_group(layout_id, &body.object_ids, body.id)
        .unwrap()
        .ok_or_else(|| AppError::bad_request("group needs at least two objects in the layout"))?;
    Ok(Json(object_group_view(group)))
}

/// Ungroup without touching member geometry/styles (#75).
pub(crate) async fn delete_object_group(
    State(st): State<AppState>,
    Path((layout_id, group_id)): Path<(i64, i64)>,
) -> AppResult<StatusCode> {
    let sol = st.sol.lock().unwrap();
    if sol.delete_object_group(layout_id, group_id).unwrap() == 0 {
        return Err(AppError::not_found());
    }
    Ok(StatusCode::OK)
}

/// One object the Create zone places (#48). `kind` is the [`ObjectKind`] string;
/// for a `field` the `field_id` names which field to bind (the server builds the
/// `Table.Field` binding + spawns the caption label per #60). `rec` is the record
/// the canvas is showing, so the returned object resolves its live value to match.
/// `props` is the optional appearance bag for a shape.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateObjectBody {
    part_id: i64,
    kind: String,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    rec: Option<i64>,
    field_id: Option<i64>,
    create_label: Option<bool>,
    content: Option<String>,
    props: Option<serde_json::Value>,
    /// The source object's binding, carried verbatim by a value-only field copy
    /// (duplicate/paste, #48/#85). Lets the copy round-trip even when the binding
    /// doesn't resolve to a live `field_id` — an empty table or an unresolved
    /// relationship path renders the object with `field_id: null`, and re-deriving
    /// the binding from `field_id` would 400. Ignored when `create_label` is true
    /// (Field-tool placement resolves the binding from `field_id` instead).
    binding: Option<String>,
    /// Owning portal for a placed column (#168/#169, Model B). When set, the new
    /// object is created as a CHILD of that portal (self-FK `parent_object_id`) and
    /// a placed `field` binds ROUTE-RELATIVE to the portal's related table (the
    /// portal's route path + the chosen related field, e.g. `sensors.reading`)
    /// rather than to the primary table. Absent (`None`) for ordinary top-level
    /// placement. FK-first: the column binds a declared route field, never authors one.
    #[serde(default)]
    parent_object_id: Option<i64>,
}

/// Create an object on a layout part from the Create zone (#48). Resolves the
/// requested record so the returned object(s) carry the same live value/label the
/// model would, and returns them as `ObjectView`s for the store to add WITHOUT a
/// re-hydrate (so the canvas's undo history survives a placement). A `field`
/// returns BOTH its value object and its spawned caption label (#60); other kinds
/// return one. 404 when the part isn't in the layout; 400 on a bad kind/field.
pub(crate) async fn create_design_object(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Json(body): Json<CreateObjectBody>,
) -> AppResult<Json<Vec<ObjectView>>> {
    let mut sol = st.sol.lock().unwrap();
    let Some((_lay, table)) = layout_table(&sol, layout_id) else {
        return Err(AppError::no_such_layout(layout_id));
    };
    let kind =
        ObjectKind::parse(&body.kind).ok_or_else(|| AppError::bad_request("bad object kind"))?;
    // `all_fields` (not `fields`, which excludes the system primary key #156):
    // the PK must be resolvable both as a field-tool choice and as a live value
    // for the object this handler just placed, now that it's manually placeable.
    let fields = sol.all_fields(table.id).unwrap();
    let by_name = by_name_for_rec(&sol, &table, &fields, body.rec);

    let created_ids: Vec<i64> = if kind == ObjectKind::Field {
        if body.create_label.unwrap_or(true) {
            // Field-tool placement: resolve the chosen field to build its binding
            // and spawn the caption label atomically (#60).
            let fid = body
                .field_id
                .ok_or_else(|| AppError::bad_request("field tool needs a fieldId"))?;
            // Placing INTO a portal (#168/#169): the field is a CHILD column bound
            // ROUTE-RELATIVE to the portal's related table. Resolve the parent
            // portal's route, pick the field from the terminal (related) table, and
            // build `<route>.<field>` (e.g. `sensors.reading`). Top-level placement
            // binds `<PrimaryTable>.<field>` against the layout's own fields.
            // `read_only` seeds from the source field's system-ness (#156) — a
            // manually-placed primary key starts read-only; every other field
            // starts editable, matching today's behavior. Portal columns bind the
            // related table's `fields()` (system PK excluded), so that branch's
            // `read_only` is always false — placing the PK as a portal column
            // stays out of scope.
            let (binding, label, read_only) = match body.parent_object_id {
                Some(parent) => {
                    let portal = sol
                        .object_by_id(layout_id, parent)
                        .unwrap()
                        .ok_or_else(|| AppError::bad_request("no such portal"))?;
                    let route_path = portal
                        .binding
                        .as_deref()
                        .filter(|b| !b.is_empty())
                        .ok_or_else(|| AppError::bad_request("portal has no route"))?;
                    let route = sol
                        .resolve_path(table.id, route_path)
                        .map_err(|e| AppError::bad_request(format!("bad portal route: {e}")))?;
                    let related = sol.fields(route.terminal_table).unwrap();
                    let f = related
                        .iter()
                        .find(|f| f.id == fid)
                        .ok_or_else(|| AppError::bad_request("no such related field"))?;
                    (format!("{route_path}.{}", f.name), f.name.clone(), f.is_system())
                }
                None => {
                    let f = fields
                        .iter()
                        .find(|f| f.id == fid)
                        .ok_or_else(|| AppError::bad_request("no such field"))?;
                    (format!("{}.{}", table.name, f.name), f.name.clone(), f.is_system())
                }
            };
            match sol
                .create_field_object(
                    layout_id,
                    body.part_id,
                    &binding,
                    &label,
                    body.x,
                    body.y,
                    body.w,
                    body.h,
                    // Portal-column containment (#168/#169): a column created inside
                    // a portal is owned by it via the self-FK; top-level is `None`.
                    body.parent_object_id,
                    read_only,
                )
                .unwrap()
            {
                Some((label_id, field_id)) => vec![label_id, field_id],
                None => return Err(AppError::not_found()),
            }
        } else {
            // Value-only field copy (duplicate/paste, #48/#85). Prefer the source
            // object's `binding` verbatim so the copy round-trips even when the
            // binding doesn't resolve to a live field_id (empty table, or an
            // unresolved relationship path) — those render with `field_id: null`,
            // and re-deriving the binding from `field_id` would 400. Fall back to
            // the field_id→binding derivation only when no binding is supplied.
            let binding = match body.binding.clone() {
                Some(b) => b,
                None => {
                    let fid = body
                        .field_id
                        .ok_or_else(|| AppError::bad_request("field tool needs a fieldId"))?;
                    let f = fields
                        .iter()
                        .find(|f| f.id == fid)
                        .ok_or_else(|| AppError::bad_request("no such field"))?;
                    format!("{}.{}", table.name, f.name)
                }
            };
            let new = NewObject {
                part_id: body.part_id,
                kind,
                x: body.x,
                y: body.y,
                w: body.w,
                h: body.h,
                binding: Some(binding),
                content: None,
                // Honor props on a value-only field create (paste, #85) so a pasted
                // field keeps its fill/border/format bag; the label-spawning branch
                // above is normal placement and has no props to carry yet.
                props: body.props.as_ref().map(|v| v.to_string()),
                // Portal-column containment (#168/#169): a value-only field copy
                // pasted into a portal is owned by it via the self-FK.
                parent_object_id: body.parent_object_id,
            };
            match sol.create_object(layout_id, &new).unwrap() {
                Some(id) => vec![id],
                None => return Err(AppError::not_found()),
            }
        }
    } else {
        let content = match kind {
            ObjectKind::Text => Some(body.content.clone().unwrap_or_default()),
            _ => None,
        };
        // A portal binds a declared relationship route: its dot-path rides the
        // same `binding` slot a field uses (#168). FK-first — the path is only
        // ever SELECTED from the layout's declared routes, never authored here.
        let binding = if kind == ObjectKind::Portal {
            body.binding.clone()
        } else {
            None
        };
        let props = body.props.as_ref().map(|v| v.to_string());
        let new = NewObject {
            part_id: body.part_id,
            kind,
            x: body.x,
            y: body.y,
            w: body.w,
            h: body.h,
            binding,
            content,
            props,
            // A non-field object (text/shape) may also be authored as a portal
            // child (#168/#169); a portal itself is always top-level.
            parent_object_id: if kind == ObjectKind::Portal {
                None
            } else {
                body.parent_object_id
            },
        };
        match sol.create_object(layout_id, &new).unwrap() {
            Some(id) => vec![id],
            None => return Err(AppError::not_found()),
        }
    };

    // Re-read the freshly inserted rows and project them exactly as the model
    // would, so the store's added object is byte-identical to a model fetch.
    let views: Vec<ObjectView> = created_ids
        .iter()
        .filter_map(|id| sol.object_by_id(layout_id, *id).unwrap())
        .map(|o| object_view(&o, &by_name))
        .collect();
    Ok(Json(views))
}

/// One object restored at its ORIGINAL id (#84). The client sends the store's
/// full `ObjectDoc` for each object it recreated on undo-of-delete / redo-of-
/// create, so the server re-inserts it byte-identically at the same id.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RestoreObjectBody {
    id: i64,
    part_id: i64,
    kind: String,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    z: i64,
    read_only: bool,
    binding: Option<String>,
    content: Option<String>,
    props: Option<String>,
    /// Owning portal for a restored column (#168/#169). Absent on older clients
    /// and for top-level objects, so it defaults to `None`.
    #[serde(default)]
    parent_object_id: Option<i64>,
}

#[derive(serde::Deserialize)]
pub(crate) struct RestoreObjectsBody {
    objects: Vec<RestoreObjectBody>,
    rec: Option<i64>,
}

/// Restore deleted objects at their ORIGINAL ids (#84 undo/redo replay) and return
/// each one's `ObjectView` resolved against `rec` — byte-identical to a model
/// fetch, so the store's already-recreated objects match the server without a
/// re-hydrate. 400 on a bad kind; 404 if a part isn't in the layout; 409 if an id
/// is already occupied (reused by an intervening create). The batch is atomic.
pub(crate) async fn restore_design_objects(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Json(body): Json<RestoreObjectsBody>,
) -> AppResult<Json<Vec<ObjectView>>> {
    let mut sol = st.sol.lock().unwrap();
    let Some((_lay, table)) = layout_table(&sol, layout_id) else {
        return Err(AppError::no_such_layout(layout_id));
    };
    let mut restores = Vec::with_capacity(body.objects.len());
    for o in &body.objects {
        let kind =
            ObjectKind::parse(&o.kind).ok_or_else(|| AppError::bad_request("bad object kind"))?;
        restores.push(RestoreObject {
            id: o.id,
            part_id: o.part_id,
            kind,
            x: o.x,
            y: o.y,
            w: o.w,
            h: o.h,
            z: o.z,
            read_only: o.read_only,
            binding: o.binding.clone(),
            content: o.content.clone(),
            props: o.props.clone(),
            parent_object_id: o.parent_object_id,
        });
    }
    match sol.restore_objects(layout_id, &restores).unwrap() {
        RestoreResult::Restored => {}
        RestoreResult::PartNotFound => return Err(AppError::not_found()),
        RestoreResult::IdInUse => return Err(AppError::conflict("id in use")),
    }
    // The record projection is identical for every restored object, so resolve
    // the fields + record once instead of per object. `all_fields`: a restored
    // object may be a manually-placed system-PK field (#156).
    let fields = sol.all_fields(table.id).unwrap();
    let by_name = by_name_for_rec(&sol, &table, &fields, body.rec);
    let mut views = Vec::with_capacity(restores.len());
    for o in &restores {
        match sol.object_by_id(layout_id, o.id).unwrap() {
            Some(object) => views.push(object_view(&object, &by_name)),
            None => return Err(AppError::not_found()),
        }
    }
    Ok(Json(views))
}

/// A band the Create zone adds (#48): the [`PartKind`] string and an optional
/// height (defaults to a workable band height).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreatePartBody {
    kind: String,
    height: Option<i64>,
}

/// The result of appending a band (#48): the new `part` plus the layout's full
/// `[{id, position}]` ordering *after* the insert. `create_part` places summary
/// bands between the body and footer and shifts the trailing parts down, so the
/// client can't guess the slot — it must resync every part's `position` from
/// `positions` (mirrors the move endpoint) or the new band renders below the
/// footer.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreatePartResult {
    part: PartView,
    positions: Vec<PartPosition>,
}

/// Append a band to a layout (#48) and return the new `PartView` plus the layout's
/// post-insert `[{id, position}]` ordering so the store places the band in its
/// server-assigned slot (summaries land above the footer). 404 unknown layout;
/// 400 bad kind.
pub(crate) async fn create_design_part(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Json(body): Json<CreatePartBody>,
) -> AppResult<Json<CreatePartResult>> {
    let sol = st.sol.lock().unwrap();
    if layout_table(&sol, layout_id).is_none() {
        return Err(AppError::no_such_layout(layout_id));
    }
    let kind = PartKind::parse(&body.kind).ok_or_else(|| AppError::bad_request("bad part kind"))?;
    let height = body.height.unwrap_or(80).max(1);
    let id = sol.create_part(layout_id, kind, height)?;
    let positions = part_positions(&sol, layout_id);
    let part = sol
        .part_by_id(layout_id, id)
        .unwrap()
        .ok_or_else(AppError::not_found)?;
    Ok(Json(CreatePartResult {
        part: part_view(&part),
        positions,
    }))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PartHeightBody {
    height: i64,
}

/// Resize a band by setting its stored height. 200 echoes the updated `PartView`;
/// 404 when no such part belongs to the layout.
pub(crate) async fn update_part_height(
    State(st): State<AppState>,
    Path((layout_id, part_id)): Path<(i64, i64)>,
    Json(body): Json<PartHeightBody>,
) -> AppResult<Json<PartView>> {
    let sol = st.sol.lock().unwrap();
    let height = body.height.max(1);
    let updated = sol.set_part_height(layout_id, part_id, height).unwrap();
    updated_part_view(&sol, layout_id, part_id, updated)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PartKindBody {
    kind: String,
}

/// Change a band's kind. 400 for an unknown kind; 404 for a foreign/unknown part.
pub(crate) async fn update_part_kind(
    State(st): State<AppState>,
    Path((layout_id, part_id)): Path<(i64, i64)>,
    Json(body): Json<PartKindBody>,
) -> AppResult<Json<PartView>> {
    let sol = st.sol.lock().unwrap();
    let kind = PartKind::parse(&body.kind).ok_or_else(|| AppError::bad_request("bad part kind"))?;
    let updated = sol.set_part_kind(layout_id, part_id, kind)?;
    updated_part_view(&sol, layout_id, part_id, updated)
}

/// Persist a band's `props` from the Band inspector (#49/Issue 7), layout-scoped,
/// and echo back the updated `PartView` (with the re-derived `part_style`) so the
/// canvas updates without a client-side re-derivation. 200 on success, 404 when no
/// such part belongs to the layout. Mirrors [`update_object_props`].
pub(crate) async fn update_part_props(
    State(st): State<AppState>,
    Path((layout_id, part_id)): Path<(i64, i64)>,
    Json(body): Json<PropsBody>,
) -> AppResult<Json<PartView>> {
    let sol = st.sol.lock().unwrap();
    let props = body.props.to_string();
    let updated = sol.set_part_props(layout_id, part_id, &props).unwrap();
    updated_part_view(&sol, layout_id, part_id, updated)
}

/// The direction a summary band moves within its layout (Issue 4).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PartMoveBody {
    up: bool,
}

/// A part's id + resolved position after a reorder — the lightweight shape the
/// move endpoint returns so the client can resync `PartDoc.position` (Issue 4).
#[derive(serde::Serialize)]
pub(crate) struct PartPosition {
    id: i64,
    position: i64,
}

/// The layout's full `[{id, position}]` ordering — the resync payload the part
/// create/move endpoints return.
fn part_positions(sol: &Solution, layout_id: i64) -> Vec<PartPosition> {
    sol.parts(layout_id)
        .unwrap()
        .into_iter()
        .map(|p| PartPosition {
            id: p.id,
            position: p.position,
        })
        .collect()
}

/// Move a summary band up/down within its layout, staying between the header and
/// footer (Issue 4). 200 returns the layout's parts as `[{id, position}]` (after
/// the move) so the client resyncs positions; 404 when the move was a no-op (no
/// such movable part / clamped at a boundary).
pub(crate) async fn move_design_part(
    State(st): State<AppState>,
    Path((layout_id, part_id)): Path<(i64, i64)>,
    Json(body): Json<PartMoveBody>,
) -> AppResult<Json<Vec<PartPosition>>> {
    let mut sol = st.sol.lock().unwrap();
    if sol.move_part(layout_id, part_id, body.up)? == 0 {
        return Err(AppError::not_found());
    }
    Ok(Json(part_positions(&sol, layout_id)))
}

/// Delete a band from a layout. Child objects are removed with it.
pub(crate) async fn delete_design_part(
    State(st): State<AppState>,
    Path((layout_id, part_id)): Path<(i64, i64)>,
) -> AppResult<StatusCode> {
    let sol = st.sol.lock().unwrap();
    if sol.delete_part(layout_id, part_id)? == 0 {
        return Err(AppError::not_found());
    }
    Ok(StatusCode::OK)
}

/// Delete an object from a layout (#48) — the Create zone's delete and the undo
/// of a create. 200 when removed, 404 when no such object belongs to the layout.
pub(crate) async fn delete_design_object(
    State(st): State<AppState>,
    Path((layout_id, object_id)): Path<(i64, i64)>,
) -> AppResult<StatusCode> {
    let sol = st.sol.lock().unwrap();
    if sol.delete_object(layout_id, object_id).unwrap() == 0 {
        return Err(AppError::not_found());
    }
    Ok(StatusCode::OK)
}

/// Delete several objects from a layout in one transaction — the bulk sibling
/// of [`delete_design_object`], mirroring [`update_objects_geometry`]: the
/// canvas POSTs `[id, …]` for a multi-delete/cut. Always 200 (unknown ids are
/// simply skipped); the body is the count actually removed, so the client can
/// detect a stale selection.
pub(crate) async fn delete_design_objects(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Json(ids): Json<Vec<i64>>,
) -> impl IntoResponse {
    let mut sol = st.sol.lock().unwrap();
    let removed = sol.delete_objects(layout_id, &ids).unwrap();
    (StatusCode::OK, removed.to_string()).into_response()
}

/// The appearance bag the Style zone commits (#49) — an opaque JSON object the
/// server stores verbatim and re-derives the shape style from on the next read.
#[derive(serde::Deserialize)]
pub(crate) struct PropsBody {
    props: serde_json::Value,
}

/// The canvas-facing result of a props commit (#49): freshly server-derived
/// styles, so the canvas updates without a client-side re-derivation.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StyleResult {
    object_style: String,
    text_style: String,
    shape_style: String,
}

/// Persist an object's `props` from the Style zone (#49), layout-scoped, and echo
/// back re-derived styles for the canvas. 200 on success, 404 when no
/// such object belongs to the layout.
pub(crate) async fn update_object_props(
    State(st): State<AppState>,
    Path((layout_id, object_id)): Path<(i64, i64)>,
    Json(body): Json<PropsBody>,
) -> AppResult<Json<StyleResult>> {
    let sol = st.sol.lock().unwrap();
    let props = body.props.to_string();
    if sol.set_object_props(layout_id, object_id, &props).unwrap() == 0 {
        return Err(AppError::not_found());
    }
    let o = sol
        .object_by_id(layout_id, object_id)
        .unwrap()
        .ok_or_else(AppError::not_found)?;
    Ok(Json(StyleResult {
        object_style: object_style(o.kind, o.props.as_deref()),
        text_style: text_style(o.kind, o.props.as_deref()),
        shape_style: if o.kind.is_shape() {
            shape_style(o.kind, o.props.as_deref())
        } else {
            String::new()
        },
    }))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BindingBody {
    field_id: i64,
    rec: Option<i64>,
}

/// Rebind a selected field object to another field on the layout's primary table.
/// The client supplies a field id rather than a raw binding so the server remains
/// the single source for the stored dot-path.
pub(crate) async fn update_object_binding(
    State(st): State<AppState>,
    Path((layout_id, object_id)): Path<(i64, i64)>,
    Json(body): Json<BindingBody>,
) -> AppResult<Json<ObjectView>> {
    let sol = st.sol.lock().unwrap();
    let Some((_lay, table)) = layout_table(&sol, layout_id) else {
        return Err(AppError::no_such_layout(layout_id));
    };
    // `all_fields`: the rebind target may be the system primary key (#156).
    let fields = sol.all_fields(table.id).unwrap();
    let field = fields
        .iter()
        .find(|f| f.id == body.field_id)
        .ok_or_else(|| AppError::bad_request("no such field"))?;
    let is_system = field.is_system();
    let binding = format!("{}.{}", table.name, field.name);
    let updated = sol
        .set_object_binding(layout_id, object_id, &binding)
        .unwrap();
    // Rebinding TO the primary key seeds read-only, same as placing it fresh
    // (#156) — the inspector's toggle can still override it afterward.
    if updated > 0 && is_system {
        sol.set_object_read_only(layout_id, object_id, true).unwrap();
    }
    updated_object_view(&sol, layout_id, object_id, body.rec, updated)
}

#[derive(serde::Deserialize)]
pub(crate) struct BindingPathBody {
    binding: String,
    rec: Option<i64>,
}

/// Set an object's binding dot-path VERBATIM (history replay of a binding diff,
/// #84). Unlike [`update_object_binding`] (keyed by `fieldId` for live field-
/// picking) this writes the already-resolved path the undo diff carries, so a
/// binding undo/redo round-trips without re-deriving from a field id.
pub(crate) async fn update_object_binding_path(
    State(st): State<AppState>,
    Path((layout_id, object_id)): Path<(i64, i64)>,
    Json(body): Json<BindingPathBody>,
) -> AppResult<Json<ObjectView>> {
    let sol = st.sol.lock().unwrap();
    if layout_table(&sol, layout_id).is_none() {
        return Err(AppError::no_such_layout(layout_id));
    }
    let updated = sol
        .set_object_binding(layout_id, object_id, &body.binding)
        .unwrap();
    updated_object_view(&sol, layout_id, object_id, body.rec, updated)
}

#[derive(serde::Deserialize)]
pub(crate) struct ContentBody {
    content: String,
}

/// Update the static content for a selected text object.
pub(crate) async fn update_object_content(
    State(st): State<AppState>,
    Path((layout_id, object_id)): Path<(i64, i64)>,
    Json(body): Json<ContentBody>,
) -> AppResult<Json<ObjectView>> {
    let sol = st.sol.lock().unwrap();
    let updated = sol
        .set_object_content(layout_id, object_id, &body.content)
        .unwrap();
    updated_object_view(&sol, layout_id, object_id, None, updated)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReadOnlyBody {
    read_only: bool,
    rec: Option<i64>,
}

/// Toggle whether a selected object renders as editable in Browse mode.
pub(crate) async fn update_object_read_only(
    State(st): State<AppState>,
    Path((layout_id, object_id)): Path<(i64, i64)>,
    Json(body): Json<ReadOnlyBody>,
) -> AppResult<Json<ObjectView>> {
    let sol = st.sol.lock().unwrap();
    let updated = sol
        .set_object_read_only(layout_id, object_id, body.read_only)
        .unwrap();
    updated_object_view(&sol, layout_id, object_id, body.rec, updated)
}
