//! Browse-mode page handlers: the home redirect, the per-layout Browse views
//! (Form/List/Table), the Layout (design) page shell, and the schema page.

use std::collections::HashMap;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Redirect},
};

use crate::edit_sessions::EditScope;
use crate::viewmodel::{
    build_bands, build_form_record, build_list, build_pending_form_record, build_pending_list_row,
    canonical_view, clamp_rec, flipbook, layout_parts_with_objects, layout_stepper, layout_table,
    table_body_columns, view_label, CellView, Chrome, DesignTemplate, FieldView, FormTemplate,
    LayoutsTemplate, ListTemplate, RecordView, SchemaTemplate, TableTemplate,
};
use crate::{format, not_found, AppState};

/// Home → the first enabled default Browse layout, preferring Form (#57/#151).
/// Keyed off "enabled default" rather than the Form view specifically, so
/// disabling a table's Form view doesn't strand the home redirect.
pub(crate) async fn index(State(st): State<AppState>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let layouts = sol.layouts().unwrap();
    // Prefer Form, then List, then Table, among enabled defaults.
    let landing = ["form", "list", "table"].iter().find_map(|&v| {
        layouts
            .iter()
            .find(|l| l.is_default && l.enabled && l.view == v)
    });
    match landing {
        Some(l) => Redirect::to(&format!("/browse/{}", l.id)).into_response(),
        // Nothing browsable (no tables yet, or every default view disabled) —
        // send them to the schema builder to create/manage tables rather than
        // a raw dead-end page (#152).
        None => Redirect::to("/schema").into_response(),
    }
}

/// Browse a layout. `?view=table|form|list` (frozen #20) picks the renderer;
/// Table is the field-derived grid, Form/List render the layout's objects.
pub(crate) async fn browse(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
    Query(q): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some((lay, table)) = layout_table(&sol, layout_id) else {
        return not_found("layout", layout_id);
    };
    // Each layout renders in its own intrinsic view; the layout id (not `?view=`)
    // selects the surface, so Form/List are independent designs (#57).
    let view = canonical_view(&lay.view);
    let mut chrome = Chrome::build(&sol, "browse", Some(&lay));

    // A pending base insert is an in-memory overlay addressed only by its edit
    // token. It is never returned by an engine found-set query (#182).
    let pending = q
        .get("edit")
        .and_then(|token| state_pending(&st, token, layout_id, table.id));

    // Found set + flipbook position drive record navigation across all views.
    let ids = sol.record_ids(&table).unwrap();
    let canonical_total = ids.len() as i64;
    let total = canonical_total + i64::from(pending.is_some());
    let rec = if pending.is_some() {
        total
    } else {
        clamp_rec(&q, total)
    };
    let current_id = if rec >= 1 && rec <= canonical_total {
        ids.get((rec - 1) as usize).copied()
    } else {
        None
    };
    chrome.nav = Some(flipbook(layout_id, view, rec, current_id, total));
    chrome.editing =
        pending.is_some() || current_id.is_some_and(|cid| st.lock_held((table.id, cid)));

    match view {
        "form" => {
            // `all_fields`: a manually-placed system-PK field object (#156) must
            // resolve its live value like any other placed field.
            let fields = sol.all_fields(table.id).unwrap();
            let record = if let Some(session) = pending.as_ref() {
                let EditScope::PendingBase { synthetic_id, .. } = session.scope else {
                    unreachable!()
                };
                Some(build_pending_form_record(
                    &sol,
                    layout_id,
                    &fields,
                    synthetic_id,
                    &session.working,
                    session.token.clone(),
                ))
            } else {
                build_form_record(&sol, layout_id, &table, &fields, &ids, rec)
            };
            Html(
                FormTemplate {
                    chrome,
                    table: table.name.clone(),
                    record,
                }
                .render()
                .unwrap(),
            )
            .into_response()
        }
        "list" => {
            // `all_fields`: see the Form branch above.
            let fields = sol.all_fields(table.id).unwrap();
            let (header, mut rows, footer) = build_list(&sol, layout_id, &table, &fields, rec);
            if let Some(session) = pending.as_ref() {
                let EditScope::PendingBase { synthetic_id, .. } = session.scope else {
                    unreachable!()
                };
                rows.push(build_pending_list_row(
                    &sol,
                    layout_id,
                    &fields,
                    synthetic_id,
                    &session.working,
                    session.token.clone(),
                ));
            }
            Html(
                ListTemplate {
                    chrome,
                    table: table.name.clone(),
                    header,
                    rows,
                    footer,
                }
                .render()
                .unwrap(),
            )
            .into_response()
        }
        _ => {
            // `all_fields`: see the Form branch above.
            let fields = sol.all_fields(table.id).unwrap();
            // One parts+objects fetch feeds both the placed columns and the bands.
            let parts = layout_parts_with_objects(&sol, layout_id);
            let columns = table_body_columns(&parts, &fields);
            let column_fields = columns.iter().map(|c| c.field.clone()).collect::<Vec<_>>();
            let records = sol.list_records(&table, &column_fields).unwrap();
            let (header, footer) = build_bands(&parts);
            let mut record_views: Vec<RecordView> = records
                .into_iter()
                .map(|r| RecordView {
                    id: r.id,
                    pending: false,
                    edit_token: String::new(),
                    cells: columns
                        .iter()
                        .zip(r.cells)
                        .map(|(c, value)| table_cell(c, value))
                        .collect(),
                })
                .collect();
            if let Some(session) = pending.as_ref() {
                let EditScope::PendingBase { synthetic_id, .. } = session.scope else {
                    unreachable!()
                };
                record_views.push(RecordView {
                    id: synthetic_id,
                    pending: true,
                    edit_token: session.token.clone(),
                    cells: columns
                        .iter()
                        .map(|column| {
                            let value = session
                                .working
                                .get(&column.field.id)
                                .cloned()
                                .unwrap_or_default();
                            table_cell(column, value)
                        })
                        .collect(),
                });
            }
            let tmpl = TableTemplate {
                chrome,
                layout_id,
                table: table.name.clone(),
                header,
                footer,
                fields: columns
                    .iter()
                    .map(|c| FieldView {
                        name: c.field.name.clone(),
                    })
                    .collect(),
                records: record_views,
            };
            Html(tmpl.render().unwrap()).into_response()
        }
    }
}

fn state_pending(
    state: &AppState,
    token: &str,
    layout_id: i64,
    table_id: i64,
) -> Option<crate::edit_sessions::EditSession> {
    state
        .edits
        .lock()
        .unwrap()
        .pending_base(token, layout_id, table_id)
}

fn table_cell(column: &crate::viewmodel::TableColumn, value: String) -> CellView {
    let mut style = format!("{}{}", column.object_style, column.text_style);
    let display = match column.format.as_ref() {
        Some(spec) => {
            let formatted = format::format_value(&value, Some(spec), column.field.kind);
            if let Some(color) = formatted.color {
                style.push_str(&format!("color:{color};"));
            }
            formatted.text
        }
        None => value.clone(),
    };
    CellView {
        field_id: column.field.id,
        value,
        display,
        style,
        read_only: column.read_only,
    }
}

/// Layout (design) mode shell. Renders the chrome + the Svelte editor mount node;
/// the canvas itself is drawn client-side by the editor, which fetches geometry
/// from [`design_model`] (#44) and renders objects from the same fields the
/// askama band macro uses, so Browse and Layout stay pixel-identical.
pub(crate) async fn design(
    State(st): State<AppState>,
    Path(layout_id): Path<i64>,
) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let Some((lay, _table)) = layout_table(&sol, layout_id) else {
        return not_found("layout", layout_id);
    };
    // One layouts() fetch feeds both the chrome and the stepper.
    let layouts = sol.layouts().unwrap_or_default();
    let mut chrome = Chrome::build_with_layouts(&layouts, "design", Some(&lay));
    // Keep the pagination control in Layout mode — repurposed to step layouts.
    chrome.nav = layout_stepper(&layouts, &lay);
    let tmpl = DesignTemplate {
        chrome,
        layout_id,
        layout: lay.name.clone(),
        view: view_label(&lay.view),
    };
    Html(tmpl.render().unwrap()).into_response()
}

/// The schema-builder page (#113). Renders the shell in `schema` mode with a
/// single mount node; the Svelte island drives everything over `/schema/*`.
pub(crate) async fn schema_page(State(st): State<AppState>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let chrome = Chrome::build(&sol, "schema", None);
    Html(SchemaTemplate { chrome }.render().unwrap()).into_response()
}

/// The Layout Manager page (#149). Renders the shell in `layouts` mode with a
/// single mount node; the Svelte island drives everything over `/layouts/*`.
pub(crate) async fn layouts_page(State(st): State<AppState>) -> impl IntoResponse {
    let sol = st.sol.lock().unwrap();
    let chrome = Chrome::build(&sol, "layouts", None);
    Html(LayoutsTemplate { chrome }.render().unwrap()).into_response()
}
