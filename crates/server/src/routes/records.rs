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
            // New mints a blank record as a DRAFT (#173): required + uniqueness
            // are deferred to the record-EXIT commit, so a table with a required
            // field can still mint a blank New record. Type/range/value-list on
            // any present value are still enforced here and surface as the same
            // 400 + message as always.
            let inserted = sol.insert_record_draft(&table, &values);
            if let Some(msg) = validation_message(&inserted) {
                return (StatusCode::BAD_REQUEST, msg).into_response();
            }
            let new_id = inserted.unwrap();
            // Register the draft so the eventual commit (or Escape) can find it.
            st.drafts.lock().unwrap().insert((table.id, new_id));
            // Land on the new record: it sorts last by id (record_ids is ORDER BY id).
            let total = sol.record_ids(&table).unwrap().len();
            let view = view_param(&form, &lay.view);
            target = format!("/browse/{layout_id}?view={view}&rec={total}");
        }
    }
    Redirect::to(&target).into_response()
}

/// Commit a record on record-EXIT: write the buffered field values under the
/// FULL validation gate, release the edit lock, and stay on the record. This is
/// the promotion point for a draft (#173): the required + uniqueness gates that
/// New deferred fire here. On a validation failure the write returns 400 and the
/// record stays a draft (and locked) — the client keeps the record open and
/// surfaces the message. On success the record is promoted (removed from the
/// draft set); for an existing (non-draft) record the remove is a harmless no-op,
/// so its behavior is unchanged. The form carries `view`/`rec` so the redirect
/// lands back on the same record in the same view (the "commit on exit" half of
/// #40).
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
            // FULL gate: a rejected write surfaces here as the same 400 + message
            // as always AND leaves the draft registered/open (early return).
            let saved = sol.update_record(&table, id, &values);
            if let Some(msg) = validation_message(&saved) {
                return (StatusCode::BAD_REQUEST, msg).into_response();
            }
            saved.unwrap();
            st.locks.lock().unwrap().remove(&(table.id, id));
            // Promote: a committed draft is now an ordinary record. No-op for a
            // record that was never a draft.
            st.drafts.lock().unwrap().remove(&(table.id, id));
            let view = view_param(&form, &lay.view);
            let rec = clamp_rec(&form, sol.record_ids(&table).unwrap().len() as i64);
            target = format!("/browse/{layout_id}?view={view}&rec={rec}");
        }
    }
    Redirect::to(&target).into_response()
}

/// Persist partial progress on a DRAFT record (#173) without committing it: run
/// the DRAFT gate (type/range/value-list on present values, but required +
/// uniqueness deferred) so the user can tab between fields — including leaving a
/// required field blank or transiently duplicating a unique value — before the
/// record-EXIT commit ([`save_record`]) runs the full gate. Does NOT touch the
/// lock or draft registries: the record stays open and a draft. A surviving rule
/// (a bad present value) still returns 400 + message; otherwise 200. Wired to the
/// per-field focus-out on a draft's inputs; existing records never post here.
pub(crate) async fn draft_save_record(
    State(st): State<AppState>,
    Path((layout_id, id)): Path<(i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some((_lay, table)) = layout_table(&sol, layout_id) else {
        return (StatusCode::NOT_FOUND, "no such layout".to_string()).into_response();
    };
    let fields = sol.fields(table.id).unwrap();
    let values = collect_values(&fields, &form);
    let saved = sol.update_record_draft(&table, id, &values);
    if let Some(msg) = validation_message(&saved) {
        return (StatusCode::BAD_REQUEST, msg).into_response();
    }
    saved.unwrap();
    (StatusCode::OK, "saved".to_string()).into_response()
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

/// Revert (the "Escape" path of #40). For an existing record this releases the
/// edit lock without writing; the client discards its buffer and reloads to the
/// committed values. For a never-committed DRAFT (#173) there is nothing to
/// revert TO — the record only ever existed as this draft — so Escape DELETES it,
/// then clears its draft + lock registrations. The client reloads onto a clamped
/// neighbor.
pub(crate) async fn revert_record(
    State(st): State<AppState>,
    Path((layout_id, id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    if let Some((_lay, table)) = layout_table(&sol, layout_id) {
        if st.is_draft((table.id, id)) {
            // A fresh draft has no prior committed state: discard the row itself.
            sol.delete_record(&table, id).unwrap();
            st.drafts.lock().unwrap().remove(&(table.id, id));
        }
        st.locks.lock().unwrap().remove(&(table.id, id));
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
    // Record-EXIT commit for the child scope: FULL gate. A 400 keeps the terminal
    // draft registered/open; success releases the lock and promotes.
    let saved = sol.update_related_record(&route, rec_id, &values);
    if let Some(msg) = validation_message(&saved) {
        return (StatusCode::BAD_REQUEST, msg).into_response();
    }
    saved.unwrap();
    st.locks.lock().unwrap().remove(&(route.terminal_table, rec_id));
    st.drafts.lock().unwrap().remove(&(route.terminal_table, rec_id));
    (StatusCode::OK, "saved").into_response()
}

/// Persist partial progress on a related (child) DRAFT (#173) without committing
/// it: the DRAFT gate over the terminal table's fields, leaving the terminal
/// row's lock + draft registration intact. Mirrors [`draft_save_record`] for a
/// portal row.
pub(crate) async fn draft_save_related_record(
    State(st): State<AppState>,
    Path((layout_id, _base_id, object_id, rec_id)): Path<(i64, i64, i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some(route) = portal_route(&sol, layout_id, object_id) else {
        return (StatusCode::NOT_FOUND, "no such portal route".to_string()).into_response();
    };
    let table = match sol.table_by_id(route.terminal_table) {
        Ok(Some(table)) => table,
        _ => return (StatusCode::NOT_FOUND, "no such related table".to_string()).into_response(),
    };
    let fields = sol.fields(route.terminal_table).unwrap_or_default();
    let values = collect_values(&fields, &form);
    let saved = sol.update_record_draft(&table, rec_id, &values);
    if let Some(msg) = validation_message(&saved) {
        return (StatusCode::BAD_REQUEST, msg).into_response();
    }
    saved.unwrap();
    (StatusCode::OK, "saved".to_string()).into_response()
}

/// Revert a related (child) record (the Escape path). For an existing terminal
/// row this releases its lock without writing. For a never-committed terminal
/// DRAFT (#173) — the row the portal's blank create-row just minted — Escape
/// DELETES the terminal record and clears its draft + lock registrations, the
/// child-scope mirror of [`revert_record`]. (The common portal case is a direct
/// to-many child, whose terminal row IS the record to discard.)
pub(crate) async fn revert_related_record(
    State(st): State<AppState>,
    Path((layout_id, _base_id, object_id, rec_id)): Path<(i64, i64, i64, i64)>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    if let Some(route) = portal_route(&sol, layout_id, object_id) {
        let key = (route.terminal_table, rec_id);
        if st.is_draft(key) {
            if let Ok(Some(table)) = sol.table_by_id(route.terminal_table) {
                sol.delete_record(&table, rec_id).unwrap();
            }
            st.drafts.lock().unwrap().remove(&key);
        }
        st.locks.lock().unwrap().remove(&key);
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
    // Mint the related record as a DRAFT (#173), the portal-row parallel of the
    // base New: required + uniqueness on the terminal are deferred to the child
    // scope's record-EXIT commit. Type/range/value-list on present values still
    // apply and surface as the same 400 + message as a base create.
    let created = sol.create_related_record_draft(&route, base_id, &values, None);
    if let Some(msg) = validation_message(&created) {
        return (StatusCode::BAD_REQUEST, msg).into_response();
    }
    match created {
        Ok(terminal_id) => {
            // Register the terminal draft so its own commit/Escape can find it.
            st.drafts
                .lock()
                .unwrap()
                .insert((route.terminal_table, terminal_id));
            (StatusCode::OK, "created".to_string()).into_response()
        }
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
