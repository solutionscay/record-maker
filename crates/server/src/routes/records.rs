//! Record action handlers (#40): create/save/open/revert/delete over the
//! Browse form contract (`f<field_id>` inputs, `view`/`rec` round-trip).

use std::collections::HashMap;

use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
};

use crate::validate::{collect_values, validation_message};
use crate::viewmodel::{clamp_rec, layout_table, view_param};
use crate::AppState;

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
