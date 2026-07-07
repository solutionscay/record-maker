//! The Layout Manager JSON API (#149): a flat, reorderable list of every
//! layout in the solution, with create/rename/delete.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use record_maker_engine::LayoutMeta;

use crate::{AppError, AppResult, AppState};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LayoutManagerView {
    id: i64,
    name: String,
    table_id: i64,
    table_name: String,
    view: String,
    position: i64,
    is_default: bool,
    enabled: bool,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateLayoutBody {
    name: String,
    table_id: i64,
    view: String,
}

#[derive(serde::Deserialize)]
pub(crate) struct RenameLayoutBody {
    name: String,
}

#[derive(serde::Deserialize)]
pub(crate) struct SetEnabledBody {
    enabled: bool,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LayoutOrderBody {
    layout_ids: Vec<i64>,
}

fn layout_manager_view(sol: &record_maker_engine::Solution, l: LayoutMeta) -> LayoutManagerView {
    let table_name = sol
        .table_by_id(l.table_id)
        .ok()
        .flatten()
        .map(|t| t.name)
        .unwrap_or_default();
    LayoutManagerView {
        id: l.id,
        name: l.name,
        table_id: l.table_id,
        table_name,
        view: l.view,
        position: l.position,
        is_default: l.is_default,
        enabled: l.enabled,
    }
}

pub(crate) async fn list_layouts(State(st): State<AppState>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let views: Vec<LayoutManagerView> = sol
        .layouts()
        .unwrap()
        .into_iter()
        .map(|l| layout_manager_view(&sol, l))
        .collect();
    Json(views)
}

pub(crate) async fn create_layout(
    State(st): State<AppState>,
    Json(body): Json<CreateLayoutBody>,
) -> AppResult<Json<LayoutManagerView>> {
    let mut sol = st.sol.lock().unwrap();
    let layout = sol.create_layout(body.table_id, &body.name, &body.view)?;
    Ok(Json(layout_manager_view(&sol, layout)))
}

pub(crate) async fn rename_layout(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<RenameLayoutBody>,
) -> AppResult<Json<LayoutManagerView>> {
    let mut sol = st.sol.lock().unwrap();
    let layout = sol
        .rename_layout(id, &body.name)?
        .ok_or_else(AppError::not_found)?;
    Ok(Json(layout_manager_view(&sol, layout)))
}

pub(crate) async fn set_layout_enabled(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<SetEnabledBody>,
) -> AppResult<Json<LayoutManagerView>> {
    let mut sol = st.sol.lock().unwrap();
    let layout = sol
        .set_layout_enabled(id, body.enabled)?
        .ok_or_else(AppError::not_found)?;
    Ok(Json(layout_manager_view(&sol, layout)))
}

pub(crate) async fn delete_layout(
    State(st): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    let mut sol = st.sol.lock().unwrap();
    if sol.delete_layout(id)? == 0 {
        return Err(AppError::not_found());
    }
    Ok(StatusCode::OK)
}

pub(crate) async fn reorder_layouts(
    State(st): State<AppState>,
    Json(body): Json<LayoutOrderBody>,
) -> AppResult<Json<Vec<LayoutManagerView>>> {
    let mut sol = st.sol.lock().unwrap();
    let layouts = sol.reorder_layouts(&body.layout_ids)?;
    Ok(Json(
        layouts
            .into_iter()
            .map(|l| layout_manager_view(&sol, l))
            .collect(),
    ))
}
