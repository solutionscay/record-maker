//! Record action handlers (#40): create/save/open/revert/delete over the
//! Browse form contract (`f<field_id>` inputs, `view`/`rec` round-trip).

use std::collections::HashMap;

use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};

use record_maker_engine::{RelatedCrudError, ResolvedRoute, Solution};

use crate::validate::{collect_values, validation_message};
use crate::viewmodel::{clamp_rec, layout_table, view_param};
use crate::AppState;

/// Resolve a portal object's bound anchor route for edit actions (#170). Looks the
/// portal object up on `layout_id`, reads its route path off the `binding` slot
/// (the same slot a field binding rides), and resolves it against the layout's
/// base table into a [`ResolvedRoute`] whose `terminal_table` is the child table a
/// related edit writes to. `None` for an unknown layout/object or a blank /
/// unresolvable binding — the caller maps that to a 4xx.
fn portal_route(sol: &Solution, layout_id: i64, object_id: i64) -> Option<ResolvedRoute> {
    let (_lay, table) = layout_table(sol, layout_id)?;
    let obj = sol.object_by_id(layout_id, object_id).ok().flatten()?;
    let binding = obj.binding.filter(|b| !b.is_empty())?;
    sol.resolve_path(table.id, &binding).ok()
}

/// Create a record from the new-record form (inputs named `f<field_id>`).
pub(crate) async fn create_record(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> impl IntoResponse {
    let mut target = format!("/browse/{layout_id}");
    {
        let sol = st.sol.lock().unwrap();
        if let Some((lay, table)) = layout_table(&sol, layout_id) {
            let fields = sol.fields(table.id).unwrap();
            let values = collect_values(&fields, &form);
            // Validation is enforced inside the engine's insert; a rejected
            // write surfaces here as the same 400 + message as always.
            let inserted = sol.insert_record(&table, &values);
            if let Some(msg) = validation_message(&inserted) {
                return (StatusCode::BAD_REQUEST, msg).into_response();
            }
            inserted.unwrap();
            // Land on the new record: it sorts last by id (record_ids is ORDER BY id).
            let total = sol.record_ids(&table).unwrap().len();
            let view = view_param(&form, &lay.view);
            target = format!("/browse/{layout_id}?view={view}&rec={total}");
        }
    }
    Redirect::to(&target).into_response()
}

/// Commit a record: write the buffered field values, release the edit lock, and
/// stay on the record. The form carries `view`/`rec` so the redirect lands back
/// on the same record in the same view (the "commit on exit" half of #40).
pub(crate) async fn save_record(
    State(st): State<AppState>,
    Path((layout_id, id)): Path<(i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> impl IntoResponse {
    let mut target = format!("/browse/{layout_id}");
    {
        let sol = st.sol.lock().unwrap();
        if let Some((lay, table)) = layout_table(&sol, layout_id) {
            let fields = sol.fields(table.id).unwrap();
            let values = collect_values(&fields, &form);
            // Validation is enforced inside the engine's update; a rejected
            // write surfaces here as the same 400 + message as always.
            let saved = sol.update_record(&table, id, &values);
            if let Some(msg) = validation_message(&saved) {
                return (StatusCode::BAD_REQUEST, msg).into_response();
            }
            saved.unwrap();
            st.locks.lock().unwrap().remove(&(table.id, id));
            let view = view_param(&form, &lay.view);
            let rec = clamp_rec(&form, sol.record_ids(&table).unwrap().len() as i64);
            target = format!("/browse/{layout_id}?view={view}&rec={rec}");
        }
    }
    Redirect::to(&target).into_response()
}

/// Open a record for editing: acquire its in-process lock. 200 once held (the
/// single session may re-open its own lock); 409 if held elsewhere (multi-user,
/// not reachable yet); 404 for an unknown layout. The "open on focus" half of #40.
pub(crate) async fn open_record(
    State(st): State<AppState>,
    Path((layout_id, id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let table_id = {
        let sol = st.sol.lock().unwrap();
        match layout_table(&sol, layout_id) {
            Some((_lay, table)) => table.id,
            None => return (StatusCode::NOT_FOUND, "no such layout"),
        }
    };
    st.locks.lock().unwrap().insert((table_id, id));
    (StatusCode::OK, "open")
}

/// Revert: release the edit lock without writing (the "Escape" path of #40). The
/// client discards its buffer and reloads to the committed values.
pub(crate) async fn revert_record(
    State(st): State<AppState>,
    Path((layout_id, id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    if let Some(table_id) = {
        let sol = st.sol.lock().unwrap();
        layout_table(&sol, layout_id).map(|(_lay, table)| table.id)
    } {
        st.locks.lock().unwrap().remove(&(table_id, id));
    }
    (StatusCode::OK, "reverted")
}

/// Delete a record, then back to the same view near where you were. The form
/// carries the current `view` and `rec` so the redirect can preserve both.
pub(crate) async fn delete_record(
    State(st): State<AppState>,
    Path((layout_id, id)): Path<(i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> impl IntoResponse {
    let mut target = format!("/browse/{layout_id}");
    {
        let sol = st.sol.lock().unwrap();
        if let Some((lay, table)) = layout_table(&sol, layout_id) {
            sol.delete_record(&table, id).unwrap();
            let total = sol.record_ids(&table).unwrap().len() as i64;
            let view = view_param(&form, &lay.view);
            target = if total > 0 {
                // Stay put if possible; clamp into the now-shorter found set.
                let rec = clamp_rec(&form, total);
                format!("/browse/{layout_id}?view={view}&rec={rec}")
            } else {
                format!("/browse/{layout_id}?view={view}")
            };
        }
    }
    Redirect::to(&target)
}

// ---------------------------------------------------------------------------
// Portal related-record inline edit (#170).
//
// A portal row is a `.rec-edit` scope whose open/commit/revert re-use the base
// record lifecycle above, re-pointed at a CHILD (terminal) record through the
// engine's related layer (`update_related_record`). The route is derived from the
// portal object's bound anchor; the lock registry is the same `(table_id, id)`
// HashSet, now keyed on the terminal table + terminal row. `base_id` rides the URL
// for symmetry with create/delete (#171/#172) but an update needs only the route
// and the terminal row id. Commits arrive via `sendBeacon` (no response is read),
// so — like the base save — validation failures return the same 400 + message and
// otherwise the write is unwrapped.
// ---------------------------------------------------------------------------

/// Open a related (child) record for editing: acquire the terminal row's lock.
/// 200 once held; 404 for an unknown layout/portal or unresolvable route.
pub(crate) async fn open_related_record(
    State(st): State<AppState>,
    Path((layout_id, _base_id, object_id, rec_id)): Path<(i64, i64, i64, i64)>,
) -> impl IntoResponse {
    let terminal_table = {
        let sol = st.sol.lock().unwrap();
        match portal_route(&sol, layout_id, object_id) {
            Some(route) => route.terminal_table,
            None => return (StatusCode::NOT_FOUND, "no such portal route"),
        }
    };
    st.locks.lock().unwrap().insert((terminal_table, rec_id));
    (StatusCode::OK, "open")
}

/// Commit a related (child) record: write the buffered terminal-field values via
/// `update_related_record`, then release the lock. Mirrors [`save_record`], scoped
/// to the child row. Inputs are the same `f<field_id>` contract, here over the
/// TERMINAL table's fields.
pub(crate) async fn save_related_record(
    State(st): State<AppState>,
    Path((layout_id, _base_id, object_id, rec_id)): Path<(i64, i64, i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some(route) = portal_route(&sol, layout_id, object_id) else {
        return (StatusCode::NOT_FOUND, "no such portal route".to_string()).into_response();
    };
    let fields = match sol.fields(route.terminal_table) {
        Ok(fields) => fields,
        Err(_) => return (StatusCode::NOT_FOUND, "no such related table").into_response(),
    };
    let values = collect_values(&fields, &form);
    let saved = sol.update_related_record(&route, rec_id, &values);
    if let Some(msg) = validation_message(&saved) {
        return (StatusCode::BAD_REQUEST, msg).into_response();
    }
    saved.unwrap();
    st.locks.lock().unwrap().remove(&(route.terminal_table, rec_id));
    (StatusCode::OK, "saved").into_response()
}

/// Revert a related (child) record: release the terminal row's lock without
/// writing (the Escape path). Mirrors [`revert_record`].
pub(crate) async fn revert_related_record(
    State(st): State<AppState>,
    Path((layout_id, _base_id, object_id, rec_id)): Path<(i64, i64, i64, i64)>,
) -> impl IntoResponse {
    if let Some(route) = {
        let sol = st.sol.lock().unwrap();
        portal_route(&sol, layout_id, object_id)
    } {
        st.locks
            .lock()
            .unwrap()
            .remove(&(route.terminal_table, rec_id));
    }
    (StatusCode::OK, "reverted")
}

/// Create a related (child) record from a portal's trailing blank row (#171):
/// mint the terminal record (and, for a join-table M:N, its join row) through the
/// engine's `create_related_record`, associating it to base record `base_id` via
/// the portal object's anchor route. CREATE-NEW ONLY — `associate_existing` is
/// `None`; associate-existing is deferred (needs the #100 value-list picker).
///
/// The gate is the relationship's, not the portal's: the route must be
/// create-determined (#11) and the anchoring relationship's `allow_create` (#110)
/// on. The render suppresses the blank row otherwise (#171 gate in `resolve_portal`);
/// this handler enforces it again (defense-in-depth), mapping a refusal to a
/// precise 4xx. Inputs are the same `f<field_id>` contract over the TERMINAL
/// table's fields; the anchor FK is stamped by the engine, so a stray FK input is
/// harmless. The client reloads on success to surface the new row + a fresh blank.
pub(crate) async fn create_related_record(
    State(st): State<AppState>,
    Path((layout_id, base_id, object_id)): Path<(i64, i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some(route) = portal_route(&sol, layout_id, object_id) else {
        return (StatusCode::NOT_FOUND, "no such portal route".to_string()).into_response();
    };
    let fields = match sol.fields(route.terminal_table) {
        Ok(fields) => fields,
        Err(_) => {
            return (StatusCode::NOT_FOUND, "no such related table".to_string()).into_response()
        }
    };
    let values = collect_values(&fields, &form);
    let created = sol.create_related_record(&route, base_id, &values, None);
    // Validation rejection surfaces as the same 400 + message as a base create.
    if let Some(msg) = validation_message(&created) {
        return (StatusCode::BAD_REQUEST, msg).into_response();
    }
    match created {
        Ok(_) => (StatusCode::OK, "created".to_string()).into_response(),
        // A refusal here means the affordance rendered despite the gate (or a
        // crafted request): map each RelatedCrudError to a precise status.
        Err(e) => {
            let status = match e.downcast_ref::<RelatedCrudError>() {
                Some(RelatedCrudError::CreateNotAllowed) => StatusCode::FORBIDDEN,
                Some(RelatedCrudError::CreateUndetermined)
                | Some(RelatedCrudError::NotARelatedRoute) => StatusCode::CONFLICT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (status, e.to_string()).into_response()
        }
    }
}

/// Delete (or unlink) a related record from a portal row (#172): the engine's
/// `delete_related_record` removes the NEAREST record for the portal's anchor
/// route — a direct to-many child is deleted, a forward to-one clears the base FK
/// (unlink; the parent survives), a join-table M:N removes only the join row (the
/// terminal survives). Never cascades.
///
/// The gate is the relationship's, not the portal's: the anchoring relationship's
/// `allow_delete` (#110) must be on. The render suppresses the per-row affordance
/// otherwise (the row's `delete_url` is empty in `resolve_portal`); this handler
/// enforces the same gate again (defense-in-depth), mapping a refusal to a precise
/// 4xx. On success the terminal row's edit lock (if any) is released and the
/// client reloads to surface the shortened set.
pub(crate) async fn delete_related_record(
    State(st): State<AppState>,
    Path((layout_id, base_id, object_id, rec_id)): Path<(i64, i64, i64, i64)>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some(route) = portal_route(&sol, layout_id, object_id) else {
        return (StatusCode::NOT_FOUND, "no such portal route".to_string()).into_response();
    };
    match sol.delete_related_record(&route, base_id, rec_id) {
        Ok(()) => {
            st.locks
                .lock()
                .unwrap()
                .remove(&(route.terminal_table, rec_id));
            (StatusCode::OK, "deleted".to_string()).into_response()
        }
        // A refusal here means the affordance rendered despite the gate (or a
        // crafted request): map each RelatedCrudError to a precise status.
        Err(e) => {
            let status = match e.downcast_ref::<RelatedCrudError>() {
                Some(RelatedCrudError::DeleteNotAllowed) => StatusCode::FORBIDDEN,
                Some(RelatedCrudError::NotARelatedRoute) => StatusCode::CONFLICT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (status, e.to_string()).into_response()
        }
    }
}
