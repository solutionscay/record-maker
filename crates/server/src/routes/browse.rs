//! Browse-mode page handlers: the home redirect, the per-layout Browse views
//! (Form/List/Table), the Layout (design) page shell, and the schema page.

use std::collections::HashMap;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Redirect},
};

use crate::viewmodel::{
    build_bands, build_form_record, build_list, canonical_view, clamp_rec, flipbook,
    layout_parts_with_objects, layout_stepper, layout_table, table_body_columns, view_label,
    CellView, Chrome, DesignTemplate, FieldView, FormTemplate, LayoutsTemplate, ListTemplate,
    RecordView, SchemaTemplate, TableTemplate,
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

    // Found set + flipbook position drive record navigation across all views.
    let ids = sol.record_ids(&table).unwrap();
    let total = ids.len() as i64;
    let rec = clamp_rec(&q, total);
    let current_id = if rec >= 1 {
        ids.get((rec - 1) as usize).copied()
    } else {
        None
    };
    chrome.nav = Some(flipbook(layout_id, view, rec, current_id, total));
    chrome.editing = current_id.is_some_and(|cid| st.lock_held((table.id, cid)));

    // Snapshot the in-process DRAFT set for this request (#173): base rows key on
    // `table.id`, portal rows on their terminal table, so the renderers get the
    // whole set and match per record. A record in here renders `data-draft` on its
    // `.rec-edit` so the client runs the lenient draft edit loop + gated commit.
    let drafts = st.drafts.lock().unwrap().clone();

    match view {
        "form" => {
            let fields = sol.fields(table.id).unwrap();
            let record = build_form_record(&sol, layout_id, &table, &fields, &ids, rec, &drafts);
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
            let fields = sol.fields(table.id).unwrap();
            let (header, rows, footer) = build_list(&sol, layout_id, &table, &fields, rec, &drafts);
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
            let fields = sol.fields(table.id).unwrap();
            // One parts+objects fetch feeds both the placed columns and the bands.
            let parts = layout_parts_with_objects(&sol, layout_id);
            let columns = table_body_columns(&parts, &fields);
            let column_fields = columns.iter().map(|c| c.field.clone()).collect::<Vec<_>>();
            let records = sol.list_records(&table, &column_fields).unwrap();
            let (header, footer) = build_bands(&parts);
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
                records: records
                    .into_iter()
                    .map(|r| RecordView {
                        id: r.id,
                        draft: drafts.contains(&(table.id, r.id)),
                        cells: columns
                            .iter()
                            .zip(r.cells)
                            .map(|(c, value)| {
                                // Format the DISPLAY value only; the input still
                                // commits the raw `value` (see _band controller).
                                let (display, style) = match c.format.as_ref() {
                                    Some(spec) => {
                                        let fmt =
                                            format::format_value(&value, Some(spec), c.field.kind);
                                        let style = fmt
                                            .color
                                            .map(|c| format!("color:{c};"))
                                            .unwrap_or_default();
                                        (fmt.text, style)
                                    }
                                    None => (value.clone(), String::new()),
                                };
                                CellView {
                                    field_id: c.field.id,
                                    value,
                                    display,
                                    style,
                                }
                            })
                            .collect(),
                    })
                    .collect(),
            };
            Html(tmpl.render().unwrap()).into_response()
        }
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
