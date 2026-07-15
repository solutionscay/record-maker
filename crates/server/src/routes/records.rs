//! Record edit-session actions (#182): owned open, atomic whole-record commit,
//! and revert over canonical rows or in-memory pending inserts.

use std::collections::HashMap;

use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    Json,
};
use record_maker_engine::{FieldMeta, RelatedCrudError, ResolvedRoute, Solution, ValidationError};
use serde::Serialize;

use crate::edit_sessions::{EditScope, EditSession, SessionError};
use crate::validate::collect_values;
use crate::viewmodel::{clamp_rec, layout_table, view_param};
use crate::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenResponse {
    ok: bool,
    edit_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    synthetic_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    redirect: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommitResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    record_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    redirect: Option<String>,
}

#[derive(Serialize)]
struct SimpleError {
    kind: &'static str,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationRecord {
    table_id: i64,
    record_id: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationIssue {
    field_id: i64,
    code: &'static str,
    message: String,
}

#[derive(Serialize)]
struct ValidationResponse {
    kind: &'static str,
    record: ValidationRecord,
    errors: Vec<ValidationIssue>,
}

fn json_error(status: StatusCode, kind: &'static str, message: impl Into<String>) -> Response {
    (
        status,
        Json(SimpleError {
            kind,
            message: message.into(),
        }),
    )
        .into_response()
}

fn session_error(error: SessionError) -> Response {
    let (status, kind) = match error {
        SessionError::Locked => (StatusCode::LOCKED, "lock_conflict"),
        SessionError::Unknown => (StatusCode::GONE, "stale_session"),
        SessionError::WrongOwner => (StatusCode::FORBIDDEN, "wrong_owner"),
        SessionError::WrongScope => (StatusCode::CONFLICT, "wrong_scope"),
    };
    json_error(status, kind, error.to_string())
}

fn edit_event(event: &str, token: &str) {
    eprintln!("[record-edit] event={event} token={token}");
}

fn write_error(
    error: anyhow::Error,
    table_id: i64,
    record_id: Option<i64>,
    edit_token: &str,
) -> Response {
    if let Some(validation) = error.downcast_ref::<ValidationError>() {
        if let Some(field_id) = validation.field_id() {
            edit_event("validation_rejected", edit_token);
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ValidationResponse {
                    kind: "validation",
                    record: ValidationRecord {
                        table_id,
                        record_id,
                    },
                    errors: vec![ValidationIssue {
                        field_id,
                        code: validation.code(),
                        message: validation.to_string(),
                    }],
                }),
            )
                .into_response();
        }
    }
    edit_event("commit_failed", edit_token);
    json_error(StatusCode::CONFLICT, "commit_failed", error.to_string())
}

fn owner(form: &HashMap<String, String>) -> Result<String, Response> {
    form.get("_owner")
        .or_else(|| form.get("owner"))
        .filter(|s| !s.is_empty())
        .cloned()
        .ok_or_else(|| {
            json_error(
                StatusCode::BAD_REQUEST,
                "missing_owner",
                "missing edit owner",
            )
        })
}

fn token(form: &HashMap<String, String>) -> Result<String, Response> {
    form.get("_edit")
        .or_else(|| form.get("edit"))
        .filter(|s| !s.is_empty())
        .cloned()
        .ok_or_else(|| {
            json_error(
                StatusCode::BAD_REQUEST,
                "missing_session",
                "missing edit session",
            )
        })
}

fn values_map(fields: &[FieldMeta], cells: Vec<String>) -> HashMap<i64, String> {
    fields
        .iter()
        .zip(cells)
        .map(|(field, value)| (field.id, value))
        .collect()
}

fn blank_values(fields: &[FieldMeta]) -> HashMap<i64, String> {
    fields
        .iter()
        .map(|field| (field.id, String::new()))
        .collect()
}

fn submitted(fields: &[FieldMeta], form: &HashMap<String, String>) -> Vec<(i64, String)> {
    collect_values(fields, form)
        .into_iter()
        .map(|(field, value)| (field.id, value))
        .collect()
}

fn working_values<'a>(
    fields: &'a [FieldMeta],
    session: &EditSession,
) -> Vec<(&'a FieldMeta, String)> {
    fields
        .iter()
        .filter_map(|field| {
            session
                .working
                .get(&field.id)
                .map(|value| (field, value.clone()))
        })
        .collect()
}

fn portal_route(sol: &Solution, layout_id: i64, object_id: i64) -> Option<ResolvedRoute> {
    let (_layout, table) = layout_table(sol, layout_id)?;
    let object = sol.object_by_id(layout_id, object_id).ok().flatten()?;
    if !object.kind.is_portal() {
        return None;
    }
    let binding = object.binding.filter(|binding| !binding.is_empty())?;
    sol.resolve_path(table.id, &binding).ok()
}

fn portal_route_contains(
    sol: &Solution,
    route: &ResolvedRoute,
    base_id: i64,
    record_id: i64,
) -> bool {
    sol.route_record_set(route, base_id)
        .is_ok_and(|ids| ids.contains(&record_id))
}

fn portal_route_is_editable(route: &ResolvedRoute) -> bool {
    route.class.create_determined()
}

/// Progressive-enhancement fallback for New. The coordinator uses
/// [`begin_new_record`] so it can read JSON without leaving the current page.
pub(crate) async fn create_record(
    State(state): State<AppState>,
    Path(layout_id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let owner = form
        .get("_owner")
        .cloned()
        .unwrap_or_else(|| format!("fallback-{layout_id}"));
    let sol = state.sol.lock().unwrap();
    let Some((_layout, table)) = layout_table(&sol, layout_id) else {
        return json_error(StatusCode::NOT_FOUND, "not_found", "no such layout");
    };
    let fields = match sol.all_fields(table.id) {
        Ok(fields) => fields,
        Err(error) => return json_error(StatusCode::CONFLICT, "open_failed", error.to_string()),
    };
    let session = state.edits.lock().unwrap().begin_pending_base(
        owner,
        layout_id,
        table.id,
        blank_values(&fields),
    );
    Redirect::to(&format!("/browse/{layout_id}?edit={}", session.token)).into_response()
}

/// Start a synthetic new-record working copy without inserting a user row.
pub(crate) async fn begin_new_record(
    State(state): State<AppState>,
    Path(layout_id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let owner = match owner(&form) {
        Ok(owner) => owner,
        Err(response) => return response,
    };
    let sol = state.sol.lock().unwrap();
    let Some((_layout, table)) = layout_table(&sol, layout_id) else {
        return json_error(StatusCode::NOT_FOUND, "not_found", "no such layout");
    };
    let fields = match sol.all_fields(table.id) {
        Ok(fields) => fields,
        Err(error) => return json_error(StatusCode::CONFLICT, "open_failed", error.to_string()),
    };
    let session = state.edits.lock().unwrap().begin_pending_base(
        owner,
        layout_id,
        table.id,
        blank_values(&fields),
    );
    let EditScope::PendingBase { synthetic_id, .. } = session.scope else {
        unreachable!()
    };
    Json(OpenResponse {
        ok: true,
        edit_token: session.token.clone(),
        synthetic_id: Some(synthetic_id),
        redirect: Some(format!("/browse/{layout_id}?edit={}", session.token)),
    })
    .into_response()
}

/// Acquire an owned lock and snapshot every field on an existing base record.
pub(crate) async fn open_record(
    State(state): State<AppState>,
    Path((layout_id, record_id)): Path<(i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let owner = match owner(&form) {
        Ok(owner) => owner,
        Err(response) => return response,
    };
    let sol = state.sol.lock().unwrap();
    let Some((_layout, table)) = layout_table(&sol, layout_id) else {
        return json_error(StatusCode::NOT_FOUND, "not_found", "no such layout");
    };
    let fields = match sol.all_fields(table.id) {
        Ok(fields) => fields,
        Err(error) => return json_error(StatusCode::CONFLICT, "open_failed", error.to_string()),
    };
    let cells = match sol.get_record(&table, &fields, record_id) {
        Ok(Some(cells)) => cells,
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "not_found", "no such record"),
        Err(error) => return json_error(StatusCode::CONFLICT, "open_failed", error.to_string()),
    };
    let scope = EditScope::Base {
        layout_id,
        table_id: table.id,
        record_id,
    };
    let session =
        match state
            .edits
            .lock()
            .unwrap()
            .begin_existing(owner, scope, values_map(&fields, cells))
        {
            Ok(session) => session,
            Err(error) => return session_error(error),
        };
    Json(OpenResponse {
        ok: true,
        edit_token: session.token,
        synthetic_id: None,
        redirect: None,
    })
    .into_response()
}

/// Commit an existing or synthetic base record. Validation rejection retains
/// the session and its lock; only a successful transaction releases it.
pub(crate) async fn save_record(
    State(state): State<AppState>,
    Path((layout_id, record_id)): Path<(i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let owner = match owner(&form) {
        Ok(owner) => owner,
        Err(response) => return response,
    };
    let edit_token = match token(&form) {
        Ok(token) => token,
        Err(response) => return response,
    };
    let sol = state.sol.lock().unwrap();
    let Some((_layout, table)) = layout_table(&sol, layout_id) else {
        return json_error(StatusCode::NOT_FOUND, "not_found", "no such layout");
    };
    let fields = match sol.all_fields(table.id) {
        Ok(fields) => fields,
        Err(error) => return json_error(StatusCode::CONFLICT, "commit_failed", error.to_string()),
    };

    let pending_scope = EditScope::PendingBase {
        layout_id,
        table_id: table.id,
        synthetic_id: record_id,
    };
    let existing_scope = EditScope::Base {
        layout_id,
        table_id: table.id,
        record_id,
    };
    let scope = if record_id < 0 {
        pending_scope
    } else {
        existing_scope
    };
    let session = match state.edits.lock().unwrap().overlay(
        &edit_token,
        &owner,
        &scope,
        submitted(&fields, &form),
    ) {
        Ok(session) => session,
        Err(error) => return session_error(error),
    };
    let values = working_values(&fields, &session);
    edit_event("commit_attempt", &edit_token);

    let committed_id = if record_id < 0 {
        match sol.commit_insert_record(&table, &values) {
            Ok(id) => id,
            Err(error) => return write_error(error, table.id, None, &edit_token),
        }
    } else {
        let current = match sol.get_record(&table, &fields, record_id) {
            Ok(Some(cells)) => values_map(&fields, cells),
            Ok(None) => {
                return json_error(StatusCode::GONE, "stale_record", "record no longer exists")
            }
            Err(error) => {
                return json_error(StatusCode::CONFLICT, "commit_failed", error.to_string())
            }
        };
        if current != session.original {
            return json_error(
                StatusCode::CONFLICT,
                "stale_record",
                "record changed after this edit session opened",
            );
        }
        if let Err(error) = sol.commit_update_record(&table, record_id, &values) {
            return write_error(error, table.id, Some(record_id), &edit_token);
        }
        record_id
    };

    if let Err(error) = state
        .edits
        .lock()
        .unwrap()
        .release(&edit_token, &owner, &scope)
    {
        return session_error(error);
    }
    edit_event("commit_succeeded", &edit_token);
    let total = sol.record_ids(&table).map(|ids| ids.len()).unwrap_or(1);
    Json(CommitResponse {
        ok: true,
        record_id: Some(committed_id),
        redirect: (record_id < 0).then(|| format!("/browse/{layout_id}?rec={total}")),
    })
    .into_response()
}

pub(crate) async fn revert_record(
    State(state): State<AppState>,
    Path((layout_id, record_id)): Path<(i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let owner = match owner(&form) {
        Ok(owner) => owner,
        Err(response) => return response,
    };
    let edit_token = match token(&form) {
        Ok(token) => token,
        Err(response) => return response,
    };
    let sol = state.sol.lock().unwrap();
    let Some((_layout, table)) = layout_table(&sol, layout_id) else {
        return json_error(StatusCode::NOT_FOUND, "not_found", "no such layout");
    };
    let scope = if record_id < 0 {
        EditScope::PendingBase {
            layout_id,
            table_id: table.id,
            synthetic_id: record_id,
        }
    } else {
        EditScope::Base {
            layout_id,
            table_id: table.id,
            record_id,
        }
    };
    if let Err(error) = state
        .edits
        .lock()
        .unwrap()
        .release(&edit_token, &owner, &scope)
    {
        return session_error(error);
    }
    edit_event("revert", &edit_token);
    Json(CommitResponse {
        ok: true,
        record_id: None,
        redirect: Some(format!("/browse/{layout_id}")),
    })
    .into_response()
}

pub(crate) async fn delete_record(
    State(state): State<AppState>,
    Path((layout_id, record_id)): Path<(i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let sol = state.sol.lock().unwrap();
    let Some((layout, table)) = layout_table(&sol, layout_id) else {
        return Redirect::to(&format!("/browse/{layout_id}")).into_response();
    };
    if state.lock_held((table.id, record_id)) {
        return json_error(
            StatusCode::LOCKED,
            "record_open",
            "commit or revert the record before deleting it",
        );
    }
    if let Err(error) = sol.delete_record(&table, record_id) {
        return json_error(StatusCode::CONFLICT, "delete_failed", error.to_string());
    }
    let total = sol
        .record_ids(&table)
        .map(|ids| ids.len() as i64)
        .unwrap_or(0);
    let view = view_param(&form, &layout.view);
    let target = if total > 0 {
        let rec = clamp_rec(&form, total);
        format!("/browse/{layout_id}?view={view}&rec={rec}")
    } else {
        format!("/browse/{layout_id}?view={view}")
    };
    Redirect::to(&target).into_response()
}

pub(crate) async fn open_related_record(
    State(state): State<AppState>,
    Path((layout_id, base_id, object_id, record_id)): Path<(i64, i64, i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let owner = match owner(&form) {
        Ok(owner) => owner,
        Err(response) => return response,
    };
    let sol = state.sol.lock().unwrap();
    let Some(route) = portal_route(&sol, layout_id, object_id) else {
        return json_error(StatusCode::NOT_FOUND, "not_found", "no such portal route");
    };
    if !portal_route_is_editable(&route) {
        return json_error(
            StatusCode::CONFLICT,
            "read_only",
            "portal route is read-only",
        );
    }
    if !portal_route_contains(&sol, &route, base_id, record_id) {
        return json_error(
            StatusCode::NOT_FOUND,
            "not_found",
            "record is not in portal route",
        );
    }
    let table = match sol.table_by_id(route.terminal_table) {
        Ok(Some(table)) => table,
        _ => return json_error(StatusCode::NOT_FOUND, "not_found", "no such related table"),
    };
    let fields = match sol.all_fields(table.id) {
        Ok(fields) => fields,
        Err(error) => return json_error(StatusCode::CONFLICT, "open_failed", error.to_string()),
    };
    let cells = match sol.get_record(&table, &fields, record_id) {
        Ok(Some(cells)) => cells,
        _ => return json_error(StatusCode::NOT_FOUND, "not_found", "no such related record"),
    };
    let scope = EditScope::Related {
        layout_id,
        base_id,
        object_id,
        table_id: route.terminal_table,
        record_id,
    };
    let session =
        match state
            .edits
            .lock()
            .unwrap()
            .begin_existing(owner, scope, values_map(&fields, cells))
        {
            Ok(session) => session,
            Err(error) => return session_error(error),
        };
    Json(OpenResponse {
        ok: true,
        edit_token: session.token,
        synthetic_id: None,
        redirect: None,
    })
    .into_response()
}

pub(crate) async fn save_related_record(
    State(state): State<AppState>,
    Path((layout_id, base_id, object_id, record_id)): Path<(i64, i64, i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let owner = match owner(&form) {
        Ok(owner) => owner,
        Err(response) => return response,
    };
    let edit_token = match token(&form) {
        Ok(token) => token,
        Err(response) => return response,
    };
    let sol = state.sol.lock().unwrap();
    let Some(route) = portal_route(&sol, layout_id, object_id) else {
        return json_error(StatusCode::NOT_FOUND, "not_found", "no such portal route");
    };
    if !portal_route_is_editable(&route) || !portal_route_contains(&sol, &route, base_id, record_id)
    {
        return json_error(
            StatusCode::CONFLICT,
            "invalid_scope",
            "related record is not editable here",
        );
    }
    let table = match sol.table_by_id(route.terminal_table) {
        Ok(Some(table)) => table,
        _ => return json_error(StatusCode::NOT_FOUND, "not_found", "no such related table"),
    };
    let fields = match sol.all_fields(table.id) {
        Ok(fields) => fields,
        Err(error) => return json_error(StatusCode::CONFLICT, "commit_failed", error.to_string()),
    };
    let scope = EditScope::Related {
        layout_id,
        base_id,
        object_id,
        table_id: route.terminal_table,
        record_id,
    };
    let session = match state.edits.lock().unwrap().overlay(
        &edit_token,
        &owner,
        &scope,
        submitted(&fields, &form),
    ) {
        Ok(session) => session,
        Err(error) => return session_error(error),
    };
    let current = match sol.get_record(&table, &fields, record_id) {
        Ok(Some(cells)) => values_map(&fields, cells),
        _ => {
            return json_error(
                StatusCode::GONE,
                "stale_record",
                "related record no longer exists",
            )
        }
    };
    if current != session.original {
        return json_error(
            StatusCode::CONFLICT,
            "stale_record",
            "related record changed after open",
        );
    }
    let values = working_values(&fields, &session);
    edit_event("commit_attempt", &edit_token);
    if let Err(error) = sol.commit_related_record_update(&route, record_id, &values) {
        return write_error(error, route.terminal_table, Some(record_id), &edit_token);
    }
    if let Err(error) = state
        .edits
        .lock()
        .unwrap()
        .release(&edit_token, &owner, &scope)
    {
        return session_error(error);
    }
    edit_event("commit_succeeded", &edit_token);
    Json(CommitResponse {
        ok: true,
        record_id: Some(record_id),
        redirect: None,
    })
    .into_response()
}

pub(crate) async fn revert_related_record(
    State(state): State<AppState>,
    Path((layout_id, base_id, object_id, record_id)): Path<(i64, i64, i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let owner = match owner(&form) {
        Ok(owner) => owner,
        Err(response) => return response,
    };
    let edit_token = match token(&form) {
        Ok(token) => token,
        Err(response) => return response,
    };
    let sol = state.sol.lock().unwrap();
    let Some(route) = portal_route(&sol, layout_id, object_id) else {
        return json_error(StatusCode::NOT_FOUND, "not_found", "no such portal route");
    };
    let scope = EditScope::Related {
        layout_id,
        base_id,
        object_id,
        table_id: route.terminal_table,
        record_id,
    };
    if let Err(error) = state
        .edits
        .lock()
        .unwrap()
        .release(&edit_token, &owner, &scope)
    {
        return session_error(error);
    }
    edit_event("revert", &edit_token);
    Json(CommitResponse {
        ok: true,
        record_id: None,
        redirect: None,
    })
    .into_response()
}

/// Open the trailing portal row as an in-memory related insert working copy.
pub(crate) async fn open_new_related_record(
    State(state): State<AppState>,
    Path((layout_id, base_id, object_id)): Path<(i64, i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let owner = match owner(&form) {
        Ok(owner) => owner,
        Err(response) => return response,
    };
    let sol = state.sol.lock().unwrap();
    let Some(route) = portal_route(&sol, layout_id, object_id) else {
        return json_error(StatusCode::NOT_FOUND, "not_found", "no such portal route");
    };
    if !route.class.create_determined() {
        return json_error(
            StatusCode::CONFLICT,
            "create_undetermined",
            "portal route cannot create a determined record",
        );
    }
    let create_allowed = route
        .hops
        .first()
        .and_then(|hop| sol.relationship_by_id(hop.relationship_id).ok().flatten())
        .is_some_and(|relationship| relationship.allow_create);
    if !create_allowed {
        return json_error(
            StatusCode::FORBIDDEN,
            "create_not_allowed",
            "related create is not allowed",
        );
    }
    let fields = match sol.all_fields(route.terminal_table) {
        Ok(fields) => fields,
        Err(error) => return json_error(StatusCode::CONFLICT, "open_failed", error.to_string()),
    };
    let session = state.edits.lock().unwrap().begin_pending_related(
        owner,
        layout_id,
        base_id,
        object_id,
        route.terminal_table,
        blank_values(&fields),
    );
    Json(OpenResponse {
        ok: true,
        edit_token: session.token,
        synthetic_id: None,
        redirect: None,
    })
    .into_response()
}

/// Commit the trailing portal row. No terminal/join/FK row exists before this
/// request; the engine creates the complete related operation atomically.
pub(crate) async fn create_related_record(
    State(state): State<AppState>,
    Path((layout_id, base_id, object_id)): Path<(i64, i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let owner = match owner(&form) {
        Ok(owner) => owner,
        Err(response) => return response,
    };
    let edit_token = match token(&form) {
        Ok(token) => token,
        Err(response) => return response,
    };
    let sol = state.sol.lock().unwrap();
    let Some(route) = portal_route(&sol, layout_id, object_id) else {
        return json_error(StatusCode::NOT_FOUND, "not_found", "no such portal route");
    };
    let fields = match sol.all_fields(route.terminal_table) {
        Ok(fields) => fields,
        Err(error) => return json_error(StatusCode::CONFLICT, "commit_failed", error.to_string()),
    };
    let scope = EditScope::PendingRelated {
        layout_id,
        base_id,
        object_id,
        table_id: route.terminal_table,
    };
    let session = match state.edits.lock().unwrap().overlay(
        &edit_token,
        &owner,
        &scope,
        submitted(&fields, &form),
    ) {
        Ok(session) => session,
        Err(error) => return session_error(error),
    };
    let values = working_values(&fields, &session);
    edit_event("commit_attempt", &edit_token);
    let terminal_id = match sol.create_related_record(&route, base_id, &values, None) {
        Ok(id) => id,
        Err(error) if error.downcast_ref::<ValidationError>().is_some() => {
            return write_error(error, route.terminal_table, None, &edit_token)
        }
        Err(error) => {
            edit_event("commit_failed", &edit_token);
            let status = match error.downcast_ref::<RelatedCrudError>() {
                Some(RelatedCrudError::CreateNotAllowed) => StatusCode::FORBIDDEN,
                Some(RelatedCrudError::CreateUndetermined)
                | Some(RelatedCrudError::NotARelatedRoute) => StatusCode::CONFLICT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            return json_error(status, "related_create_failed", error.to_string());
        }
    };
    if let Err(error) = state
        .edits
        .lock()
        .unwrap()
        .release(&edit_token, &owner, &scope)
    {
        return session_error(error);
    }
    edit_event("commit_succeeded", &edit_token);
    Json(CommitResponse {
        ok: true,
        record_id: Some(terminal_id),
        redirect: None,
    })
    .into_response()
}

pub(crate) async fn revert_new_related_record(
    State(state): State<AppState>,
    Path((layout_id, base_id, object_id)): Path<(i64, i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let owner = match owner(&form) {
        Ok(owner) => owner,
        Err(response) => return response,
    };
    let edit_token = match token(&form) {
        Ok(token) => token,
        Err(response) => return response,
    };
    let sol = state.sol.lock().unwrap();
    let Some(route) = portal_route(&sol, layout_id, object_id) else {
        return json_error(StatusCode::NOT_FOUND, "not_found", "no such portal route");
    };
    let scope = EditScope::PendingRelated {
        layout_id,
        base_id,
        object_id,
        table_id: route.terminal_table,
    };
    if let Err(error) = state
        .edits
        .lock()
        .unwrap()
        .release(&edit_token, &owner, &scope)
    {
        return session_error(error);
    }
    edit_event("revert", &edit_token);
    Json(CommitResponse {
        ok: true,
        record_id: None,
        redirect: None,
    })
    .into_response()
}

pub(crate) async fn delete_related_record(
    State(state): State<AppState>,
    Path((layout_id, base_id, object_id, record_id)): Path<(i64, i64, i64, i64)>,
) -> Response {
    let sol = state.sol.lock().unwrap();
    let Some(route) = portal_route(&sol, layout_id, object_id) else {
        return json_error(StatusCode::NOT_FOUND, "not_found", "no such portal route");
    };
    if state.lock_held((route.terminal_table, record_id)) {
        return json_error(
            StatusCode::LOCKED,
            "record_open",
            "commit or revert the related record first",
        );
    }
    if !portal_route_contains(&sol, &route, base_id, record_id) {
        return json_error(
            StatusCode::NOT_FOUND,
            "not_found",
            "record is not in portal route",
        );
    }
    match sol.delete_related_record(&route, base_id, record_id) {
        Ok(()) => Json(CommitResponse {
            ok: true,
            record_id: None,
            redirect: None,
        })
        .into_response(),
        Err(error) => {
            let status = match error.downcast_ref::<RelatedCrudError>() {
                Some(RelatedCrudError::DeleteNotAllowed) => StatusCode::FORBIDDEN,
                Some(RelatedCrudError::NotARelatedRoute) => StatusCode::CONFLICT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            json_error(status, "related_delete_failed", error.to_string())
        }
    }
}
