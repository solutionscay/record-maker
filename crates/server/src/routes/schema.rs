//! The schema JSON API (#107): tables, fields, relationships, and value
//! lists, plus the field-options ↔ relationship reference sync.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use record_maker_engine::{
    options::with_reference, FieldKind, FieldMeta, FieldReference, FieldReferenceError, NewField,
    NewRelationship, NewValueList, RelationshipMeta, Solution, TableMeta, ValueListItem,
    ValueListMeta,
};
use serde_json::Value;

use crate::{AppError, AppResult, AppState};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TableSchemaView {
    id: i64,
    name: String,
    notes: String,
    phys: String,
    position: i64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FieldSchemaView {
    id: i64,
    name: String,
    notes: String,
    phys: String,
    kind: String,
    options: Value,
    position: i64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RelationshipSchemaView {
    id: i64,
    name: String,
    from_table: i64,
    to_table: i64,
    from_field: i64,
    to_field: i64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ValueListView {
    id: i64,
    name: String,
    source: String,
    config: Value,
    position: i64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ValueListItemView {
    value: String,
    display: String,
    divider: bool,
}

#[derive(serde::Deserialize)]
pub(crate) struct ValueListBody {
    name: String,
    source: String,
    config: Value,
}

#[derive(serde::Deserialize)]
pub(crate) struct DuplicateValueListBody {
    name: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateTableBody {
    name: String,
    notes: Option<String>,
    fields: Option<Vec<FieldBody>>,
}

#[derive(serde::Deserialize)]
pub(crate) struct RenameBody {
    name: String,
}

#[derive(serde::Deserialize)]
pub(crate) struct FieldBody {
    name: String,
    kind: String,
    notes: Option<String>,
    options: Option<Value>,
}

#[derive(serde::Deserialize)]
pub(crate) struct UpdateTableBody {
    name: String,
    notes: Option<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct UpdateFieldBody {
    name: String,
    kind: String,
    notes: Option<String>,
    options: Option<Value>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FieldOrderBody {
    field_ids: Vec<i64>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TableOrderBody {
    table_ids: Vec<i64>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RelationshipBody {
    name: String,
    from_table: i64,
    to_table: i64,
    from_field: i64,
    to_field: i64,
}

fn table_schema_view(t: TableMeta) -> TableSchemaView {
    TableSchemaView {
        id: t.id,
        name: t.name,
        notes: t.notes,
        phys: t.phys,
        position: t.position,
    }
}

fn field_schema_view_with_options(f: FieldMeta, mut options: Value) -> FieldSchemaView {
    if let Some(obj) = options.as_object_mut() {
        if obj.get("system").and_then(|v| v.as_bool()).unwrap_or(false) {
            let validation = serde_json::json!({
                "primary": true,
                "required": true,
                "unique": true
            });
            obj.insert("validation".to_string(), validation);
        }
    }
    FieldSchemaView {
        id: f.id,
        name: f.name,
        notes: f.notes,
        phys: f.phys,
        kind: f.kind.as_str().to_string(),
        options,
        position: f.position,
    }
}

fn relationship_schema_view(r: RelationshipMeta) -> RelationshipSchemaView {
    RelationshipSchemaView {
        id: r.id,
        name: r.name,
        from_table: r.from_table,
        to_table: r.to_table,
        from_field: r.from_field,
        to_field: r.to_field,
    }
}

fn value_list_view(list: ValueListMeta) -> ValueListView {
    ValueListView {
        id: list.id,
        name: list.name,
        source: list.source,
        config: serde_json::from_str::<Value>(&list.config).unwrap_or(Value::Null),
        position: list.position,
    }
}

fn value_list_item_view(item: ValueListItem) -> ValueListItemView {
    ValueListItemView {
        value: item.value,
        display: item.display,
        divider: item.divider,
    }
}

fn value_list_body(body: ValueListBody) -> Result<NewValueList, &'static str> {
    if body.name.trim().is_empty() {
        return Err("value list name is required");
    }
    if body.source != "custom" && body.source != "field" {
        return Err("bad value list source");
    }
    let config = serde_json::to_string(&body.config).map_err(|_| "bad value list config")?;
    Ok(NewValueList {
        name: body.name,
        source: body.source,
        config,
    })
}

fn parse_new_field(f: FieldBody) -> Result<NewField, &'static str> {
    let Some(kind) = FieldKind::parse(&f.kind) else {
        return Err("bad field kind");
    };
    Ok(NewField { name: f.name, kind })
}

fn canonical_options(options: &Value) -> Result<String, &'static str> {
    if !options.is_object() {
        return Err("field options must be an object");
    }
    serde_json::to_string(options).map_err(|_| "field options must be valid JSON")
}

/// Parse the posted options bag's `reference` key into the engine's typed
/// [`FieldReference`] — the HTTP-payload-shape half of the sync; the engine
/// owns applying it ([`Solution::set_field_reference`]).
fn reference_constraint(options: &Value) -> Result<Option<FieldReference>, &'static str> {
    let Some(reference) = options.get("reference") else {
        return Ok(None);
    };
    if reference.is_null() {
        return Ok(None);
    }
    let Some(obj) = reference.as_object() else {
        return Err("field reference must be an object");
    };
    let name = obj
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("field reference needs a name")?;
    let to_table = obj
        .get("toTable")
        .and_then(Value::as_i64)
        .ok_or("field reference needs a target table")?;
    let to_field = obj
        .get("toField")
        .and_then(Value::as_i64)
        .ok_or("field reference needs a target field")?;
    Ok(Some(FieldReference {
        name: name.to_string(),
        to_table,
        to_field,
    }))
}

fn relationship_for_source_field(
    sol: &Solution,
    table_id: i64,
    field_id: i64,
) -> anyhow::Result<Option<RelationshipMeta>> {
    Ok(sol
        .relationships_from_table(table_id)?
        .into_iter()
        .find(|r| r.from_field == field_id))
}

/// A table's relationships indexed by source field (first match wins, matching
/// [`relationship_for_source_field`]'s `find`), so the whole-table field
/// listing runs one relationships query instead of one per field.
fn relationships_by_source_field(
    sol: &Solution,
    table_id: i64,
) -> std::collections::HashMap<i64, RelationshipMeta> {
    let mut map = std::collections::HashMap::new();
    for rel in sol.relationships_from_table(table_id).unwrap_or_default() {
        map.entry(rel.from_field).or_insert(rel);
    }
    map
}

fn field_options_for_schema(sol: &Solution, table_id: i64, field: &FieldMeta) -> Value {
    let options = field.options_value();
    match relationship_for_source_field(sol, table_id, field.id) {
        Ok(Some(rel)) => with_reference(options, &rel),
        _ => options,
    }
}

fn field_schema_view_for_table(sol: &Solution, table_id: i64, field: FieldMeta) -> FieldSchemaView {
    let options = field_options_for_schema(sol, table_id, &field);
    field_schema_view_with_options(field, options)
}

/// Align a field's reference constraint with the relationship store: parse the
/// posted `reference` key, then hand the whole sync to the engine's single
/// transactional [`Solution::set_field_reference`] op. The status mapping is
/// unchanged: bad payload → 400, missing source/target field → 404 (with the
/// engine error's message), anything else → 409.
fn sync_reference_constraint(
    sol: &mut Solution,
    table_id: i64,
    field_id: i64,
    options: &Value,
) -> Result<Option<RelationshipMeta>, (StatusCode, String)> {
    let reference =
        reference_constraint(options).map_err(|msg| (StatusCode::BAD_REQUEST, msg.to_string()))?;
    sol.set_field_reference(table_id, field_id, reference.as_ref())
        .map_err(|e| match e.downcast_ref::<FieldReferenceError>() {
            Some(err) => (StatusCode::NOT_FOUND, err.to_string()),
            None => (StatusCode::CONFLICT, e.to_string()),
        })
}

fn relationship_body(body: RelationshipBody) -> NewRelationship {
    NewRelationship {
        name: body.name,
        from_table: body.from_table,
        to_table: body.to_table,
        from_field: body.from_field,
        to_field: body.to_field,
    }
}

pub(crate) async fn schema_tables(State(st): State<AppState>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let views: Vec<TableSchemaView> = sol
        .tables()
        .unwrap()
        .into_iter()
        .map(table_schema_view)
        .collect();
    Json(views)
}

pub(crate) async fn create_schema_table(
    State(st): State<AppState>,
    Json(body): Json<CreateTableBody>,
) -> AppResult<Json<TableSchemaView>> {
    let notes = body.notes.unwrap_or_default();
    let fields = body
        .fields
        .unwrap_or_default()
        .into_iter()
        .map(parse_new_field)
        .collect::<Result<Vec<_>, _>>()
        .map_err(AppError::bad_request)?;
    let mut sol = st.sol.lock().unwrap();
    let table_id = sol.create_table(&body.name, &fields)?;
    let table = if notes.is_empty() {
        sol.table_by_id(table_id).unwrap().unwrap()
    } else {
        sol.update_table(table_id, &body.name, &notes)?
            .ok_or_else(AppError::not_found)?
    };
    Ok(Json(table_schema_view(table)))
}

pub(crate) async fn update_schema_table(
    State(st): State<AppState>,
    Path(table_id): Path<i64>,
    Json(body): Json<UpdateTableBody>,
) -> AppResult<Json<TableSchemaView>> {
    let mut sol = st.sol.lock().unwrap();
    let table = sol
        .update_table(table_id, &body.name, body.notes.as_deref().unwrap_or(""))?
        .ok_or_else(AppError::not_found)?;
    Ok(Json(table_schema_view(table)))
}

pub(crate) async fn rename_schema_table(
    State(st): State<AppState>,
    Path(table_id): Path<i64>,
    Json(body): Json<RenameBody>,
) -> AppResult<Json<TableSchemaView>> {
    let mut sol = st.sol.lock().unwrap();
    let table = sol
        .rename_table(table_id, &body.name)?
        .ok_or_else(AppError::not_found)?;
    Ok(Json(table_schema_view(table)))
}

pub(crate) async fn delete_schema_table(
    State(st): State<AppState>,
    Path(table_id): Path<i64>,
) -> AppResult<StatusCode> {
    let mut sol = st.sol.lock().unwrap();
    if sol.delete_table(table_id)? == 0 {
        return Err(AppError::not_found());
    }
    Ok(StatusCode::OK)
}

pub(crate) async fn schema_fields(
    State(st): State<AppState>,
    Path(table_id): Path<i64>,
) -> AppResult<Json<Vec<FieldSchemaView>>> {
    let sol = st.sol.lock().unwrap();
    if sol.table_by_id(table_id).unwrap().is_none() {
        return Err(AppError::not_found());
    }
    let rels = relationships_by_source_field(&sol, table_id);
    let views: Vec<FieldSchemaView> = sol
        .all_fields(table_id)
        .unwrap()
        .into_iter()
        .map(|field| {
            let options = field.options_value();
            let options = match rels.get(&field.id) {
                Some(rel) => with_reference(options, rel),
                None => options,
            };
            field_schema_view_with_options(field, options)
        })
        .collect();
    Ok(Json(views))
}

/// Canonicalise an optional `options` bag into `(raw value, canonical JSON)`;
/// 400 with the validation message when it doesn't parse.
fn parse_options(options: Option<&Value>) -> AppResult<Option<(Value, String)>> {
    match options {
        Some(options) => match canonical_options(options) {
            Ok(options_json) => Ok(Some((options.clone(), options_json))),
            Err(msg) => Err(AppError::bad_request(msg)),
        },
        None => Ok(None),
    }
}

/// Shared tail of [`create_schema_field`] / [`update_schema_field`]: sync the
/// reference constraint, persist the canonical options, and project the field's
/// schema view. With no options bag the field projects as-is.
fn apply_field_options(
    sol: &mut Solution,
    table_id: i64,
    field: FieldMeta,
    options: Option<(Value, String)>,
) -> AppResult<Json<FieldSchemaView>> {
    let Some((options_value, options_json)) = options else {
        return Ok(Json(field_schema_view_for_table(sol, table_id, field)));
    };
    let rel = sync_reference_constraint(sol, table_id, field.id, &options_value)?;
    let field = sol
        .update_field_options(table_id, field.id, &options_json)?
        .ok_or_else(AppError::not_found)?;
    let options = rel.as_ref().map_or_else(
        || field_options_for_schema(sol, table_id, &field),
        |rel| with_reference(field.options_value(), rel),
    );
    Ok(Json(field_schema_view_with_options(field, options)))
}

pub(crate) async fn create_schema_field(
    State(st): State<AppState>,
    Path(table_id): Path<i64>,
    Json(body): Json<FieldBody>,
) -> AppResult<Json<FieldSchemaView>> {
    let notes = body.notes.clone().unwrap_or_default();
    let options = parse_options(body.options.as_ref())?;
    let field = parse_new_field(body).map_err(AppError::bad_request)?;
    let mut sol = st.sol.lock().unwrap();
    let field = match sol.add_field(table_id, &field) {
        Ok(field) => field,
        Err(e) if e.to_string().contains("no table") => return Err(AppError::not_found()),
        Err(e) => return Err(e.into()),
    };
    let field = if notes.is_empty() {
        field
    } else {
        sol.update_field(table_id, field.id, &field.name, field.kind, &notes)?
            .ok_or_else(AppError::not_found)?
    };
    apply_field_options(&mut sol, table_id, field, options)
}

pub(crate) async fn update_schema_field(
    State(st): State<AppState>,
    Path((table_id, field_id)): Path<(i64, i64)>,
    Json(body): Json<UpdateFieldBody>,
) -> AppResult<Json<FieldSchemaView>> {
    let kind =
        FieldKind::parse(&body.kind).ok_or_else(|| AppError::bad_request("bad field kind"))?;
    let options = parse_options(body.options.as_ref())?;
    let mut sol = st.sol.lock().unwrap();
    let field = sol
        .update_field(
            table_id,
            field_id,
            &body.name,
            kind,
            body.notes.as_deref().unwrap_or(""),
        )?
        .ok_or_else(AppError::not_found)?;
    apply_field_options(&mut sol, table_id, field, options)
}

pub(crate) async fn reorder_schema_fields(
    State(st): State<AppState>,
    Path(table_id): Path<i64>,
    Json(body): Json<FieldOrderBody>,
) -> AppResult<Json<Vec<FieldSchemaView>>> {
    let mut sol = st.sol.lock().unwrap();
    if sol.table_by_id(table_id).unwrap().is_none() {
        return Err(AppError::not_found());
    }
    let _ = sol.reorder_fields(table_id, &body.field_ids)?;
    let fields = sol.all_fields(table_id)?;
    Ok(Json(
        fields
            .into_iter()
            .map(|field| field_schema_view_for_table(&sol, table_id, field))
            .collect(),
    ))
}

pub(crate) async fn reorder_schema_tables(
    State(st): State<AppState>,
    Json(body): Json<TableOrderBody>,
) -> AppResult<Json<Vec<TableSchemaView>>> {
    let mut sol = st.sol.lock().unwrap();
    let tables = sol.reorder_tables(&body.table_ids)?;
    Ok(Json(
        tables
            .into_iter()
            .map(table_schema_view)
            .collect(),
    ))
}

pub(crate) async fn delete_schema_field(
    State(st): State<AppState>,
    Path((table_id, field_id)): Path<(i64, i64)>,
) -> AppResult<StatusCode> {
    let mut sol = st.sol.lock().unwrap();
    if sol.delete_field(table_id, field_id)? == 0 {
        return Err(AppError::not_found());
    }
    Ok(StatusCode::OK)
}

pub(crate) async fn schema_relationships(State(st): State<AppState>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let views: Vec<RelationshipSchemaView> = sol
        .relationships()
        .unwrap()
        .into_iter()
        .map(relationship_schema_view)
        .collect();
    Json(views)
}

pub(crate) async fn create_schema_relationship(
    State(st): State<AppState>,
    Json(body): Json<RelationshipBody>,
) -> AppResult<Json<RelationshipSchemaView>> {
    let rel = relationship_body(body);
    let mut sol = st.sol.lock().unwrap();
    // The engine stamps the source field's options `reference` key in the same
    // transaction as the relationship row (#134).
    let rel = sol
        .create_relationship(&rel)?
        .ok_or_else(AppError::not_found)?;
    Ok(Json(relationship_schema_view(rel)))
}

pub(crate) async fn update_schema_relationship(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<RelationshipBody>,
) -> AppResult<Json<RelationshipSchemaView>> {
    let rel = relationship_body(body);
    let mut sol = st.sol.lock().unwrap();
    // The engine clears the old source field's `reference` key (when the FK
    // side moved) and stamps the new one, transactionally with the row (#134).
    let rel = sol
        .update_relationship(id, &rel)?
        .ok_or_else(AppError::not_found)?;
    Ok(Json(relationship_schema_view(rel)))
}

pub(crate) async fn delete_schema_relationship(
    State(st): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    let mut sol = st.sol.lock().unwrap();
    // The engine clears the source field's `reference` key in the same
    // transaction as the row delete (#134).
    if sol.delete_relationship(id)? == 0 {
        return Err(AppError::not_found());
    }
    Ok(StatusCode::OK)
}

pub(crate) async fn value_lists(State(st): State<AppState>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let lists = sol
        .value_lists()
        .unwrap()
        .into_iter()
        .map(value_list_view)
        .collect::<Vec<_>>();
    Json(lists)
}

pub(crate) async fn create_value_list(
    State(st): State<AppState>,
    Json(body): Json<ValueListBody>,
) -> AppResult<Json<ValueListView>> {
    let list = value_list_body(body).map_err(|_| AppError::bad_request("bad value list"))?;
    let mut sol = st.sol.lock().unwrap();
    Ok(Json(value_list_view(sol.create_value_list(&list)?)))
}

pub(crate) async fn update_value_list(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<ValueListBody>,
) -> AppResult<Json<ValueListView>> {
    let list = value_list_body(body).map_err(|_| AppError::bad_request("bad value list"))?;
    let mut sol = st.sol.lock().unwrap();
    let list = sol
        .update_value_list(id, &list)?
        .ok_or_else(AppError::not_found)?;
    Ok(Json(value_list_view(list)))
}

pub(crate) async fn duplicate_value_list(
    State(st): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<DuplicateValueListBody>,
) -> AppResult<Json<ValueListView>> {
    let mut sol = st.sol.lock().unwrap();
    let list = sol
        .duplicate_value_list(id, body.name.as_deref())?
        .ok_or_else(AppError::not_found)?;
    Ok(Json(value_list_view(list)))
}

pub(crate) async fn delete_value_list(
    State(st): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<StatusCode> {
    let sol = st.sol.lock().unwrap();
    if sol.delete_value_list(id)? == 0 {
        return Err(AppError::not_found());
    }
    Ok(StatusCode::OK)
}

pub(crate) async fn value_list_items(
    State(st): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Vec<ValueListItemView>>> {
    let sol = st.sol.lock().unwrap();
    let items = sol
        .resolve_value_list(id)?
        .ok_or_else(AppError::not_found)?;
    Ok(Json(items.into_iter().map(value_list_item_view).collect()))
}
