//! The server test suite: route-level tests over the axum router plus the
//! #44 renderer-parity goldens. Kept as the crate-root `tests` module (see
//! `lib.rs`) so test paths stay `tests::…` across the module split.

use std::collections::HashMap;

use askama::Template;
use axum::http::StatusCode;
use record_maker_engine::{
    FieldKind, NewField, NewRelationship, NewValueList, ObjectKind, ObjectMeta, PartKind, PartMeta,
    Solution,
};

use crate::viewmodel::*;
use crate::*;

/// A bare Form chrome with a flipbook present (the band only renders inside
/// the `<form>`, which requires `chrome.nav` to be `Some`).
fn form_chrome() -> Chrome {
    Chrome {
        mode: "browse",
        layouts: Vec::new(),
        current_layout: Some(1),
        view_tabs: Vec::new(),
        nav: Some(flipbook(1, "form", 1, Some(1), 1)),
        editing: false,
    }
}

fn field_obj(field_id: i64, value: &str, read_only: bool) -> ObjectView {
    ObjectView {
        id: field_id,
        kind: "field",
        field: true,
        shape: false,
        field_id: Some(field_id),
        x: 0,
        y: 0,
        w: 100,
        h: 24,
        z: 0,
        read_only,
        binding: format!("T.Field{field_id}"),
        content: String::new(),
        props: String::new(),
        object_style: String::new(),
        text_style: String::new(),
        label: format!("Field {field_id}"),
        value: value.to_string(),
        raw: value.to_string(),
        shape_style: String::new(),
    }
}

fn body_part(sol: &Solution, layout_id: i64) -> PartMeta {
    sol.parts(layout_id)
        .unwrap()
        .into_iter()
        .find(|p| p.kind == PartKind::Body)
        .expect("body part")
}

#[test]
fn unresolved_field_binding_renders_binding_fallback() {
    let object = ObjectMeta {
        id: 1,
        part_id: 1,
        kind: ObjectKind::Field,
        x: 0,
        y: 0,
        w: 100,
        h: 24,
        z: 0,
        read_only: true,
        binding: Some("Customers.Missing".into()),
        content: None,
        props: None,
    };
    let view = object_view(&object, &HashMap::new());
    assert_eq!(view.label, "Customers.Missing");
    assert_eq!(view.value, "Customers.Missing");
}

/// The #43 acceptance: a read-only object renders a non-editable value, while
/// an editable object in the same (editable) Form view renders an input.
#[test]
fn read_only_object_renders_value_editable_object_renders_input() {
    let part = PartView {
        id: 1,
        kind: "body",
        height: 60,
        props: String::new(),
        part_style: String::new(),
        objects: vec![
            field_obj(1, "EDITABLE_VAL", false),
            field_obj(2, "READONLY_VAL", true),
        ],
    };
    let tmpl = FormTemplate {
        chrome: form_chrome(),
        table: "T".into(),
        record: Some(FormRecord {
            id: 1,
            parts: vec![part],
        }),
    };
    let html = tmpl.render().unwrap();

    // Editable object → an input bound to f1 carrying its value.
    assert!(
        html.contains(r#"name="f1""#) && html.contains(r#"value="EDITABLE_VAL""#),
        "editable object should render an input"
    );
    // Read-only object → no input for f2; its value shows in a read-only span.
    assert!(
        !html.contains(r#"name="f2""#),
        "read-only object must not render an editable input"
    );
    assert!(
        html.contains("fm-readonly") && html.contains("READONLY_VAL"),
        "read-only object should render its value as a non-editable span"
    );
}

/// z-order reaches the DOM as an explicit CSS `z-index` so overlap is
/// deterministic regardless of source order.
#[test]
fn object_z_order_renders_as_css_z_index() {
    let mut o = field_obj(1, "v", false);
    o.z = 7;
    let tmpl = FormTemplate {
        chrome: form_chrome(),
        table: "T".into(),
        record: Some(FormRecord {
            id: 1,
            parts: vec![PartView {
                id: 1,
                kind: "body",
                height: 60,
                props: String::new(),
                part_style: String::new(),
                objects: vec![o],
            }],
        }),
    };
    assert!(tmpl.render().unwrap().contains("z-index:7"));
}

/// End-to-end through the real route: a default form is all-editable, but
/// once a field object is flagged read-only the Browse Form view stops
/// rendering an input for it (and keeps the input for editable fields) — the
/// #43 read-only flag honored by Browse, wired engine → handler → template.
#[tokio::test]
async fn browse_form_honors_per_object_read_only_end_to_end() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt; // for `oneshot`

    let mut sol = Solution::open_in_memory().unwrap();
    let tid = sol
        .create_table(
            "Customers",
            &[
                NewField {
                    name: "Name".into(),
                    kind: FieldKind::Text,
                },
                NewField {
                    name: "Email".into(),
                    kind: FieldKind::Text,
                },
            ],
        )
        .unwrap();
    let table = sol.table_by_name("Customers").unwrap().unwrap();
    let fields = sol.fields(tid).unwrap();
    let (name_fid, email_fid) = (fields[0].id, fields[1].id);
    sol.insert_record(
        &table,
        &[(&fields[0], "Ada".into()), (&fields[1], "ada@x.com".into())],
    )
    .unwrap();
    let layout_id = sol.layouts().unwrap()[0].id;
    // Flag the Name object read-only (what the Layout canvas will do, #47).
    sol.app
        .execute(
            "UPDATE meta_object SET read_only=1 WHERE binding='Customers.Name'",
            [],
        )
        .unwrap();

    let state = AppState::new(sol);
    let req = Request::builder()
        .uri(format!("/browse/{layout_id}?view=form"))
        .body(Body::empty())
        .unwrap();
    let resp = app(state).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();

    // Read-only Name: value shown, but no editable input bound to it.
    assert!(html.contains("Ada"), "read-only value still rendered");
    assert!(
        !html.contains(&format!(r#"name="f{name_fid}""#)),
        "read-only field must not render an input"
    );
    assert!(
        html.contains("fm-readonly"),
        "read-only object marked in markup"
    );
    // Editable Email: input present.
    assert!(
        html.contains(&format!(r#"name="f{email_fid}""#)) && html.contains(r#"value="ada@x.com""#),
        "editable field still renders an input"
    );
}

// ---- #44 shared-renderer parity oracle --------------------------------
//
// The Layout canvas (Svelte) renders objects from the same fields the askama
// band macro uses. These tests pin BOTH ends of that to committed goldens:
//   - `canvas.parity.html`  — the canonical band DOM (this macro is the spec).
//   - `canvas.fixture.json` — the exact `/design/:layout/model` response.
// The Svelte side (ui/) renders `LayoutPreview` from the SAME fixture JSON and
// asserts it normalizes to the SAME canvas golden, so neither renderer can
// drift. `normalize_html` is the shared contract — keep it byte-equal to the
// JS copy in `ui/scripts/parity-check.mjs`.
//
// Run `REGEN=1 cargo test -p record-maker-server` to (re)generate the goldens
// from the live macro/endpoint output after an intentional DOM change.

/// Strip HTML comments, collapse whitespace runs to one space, then drop
/// spaces adjacent to tag boundaries. This absorbs (1) Svelte 5 SSR hydration
/// markers like `<!--[-->`/`<!---->` (the macro emits none, so stripping is a
/// no-op on the Browse side) and (2) harmless indentation/newline differences,
/// while preserving text content and attribute strings. The JS copy in
/// `ui/scripts/parity-check.mjs` MUST stay byte-equivalent to this.
fn normalize_html(s: &str) -> String {
    // 1. remove `<!-- ... -->` comments.
    let mut decommented = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        match rest.find("<!--") {
            None => {
                decommented.push_str(rest);
                break;
            }
            Some(i) => {
                decommented.push_str(&rest[..i]);
                match rest[i..].find("-->") {
                    Some(j) => rest = &rest[i + j + 3..],
                    None => break,
                }
            }
        }
    }
    // 2. collapse whitespace runs to a single space.
    let mut collapsed = String::with_capacity(decommented.len());
    let mut prev_ws = false;
    for c in decommented.chars() {
        if c.is_whitespace() {
            if !prev_ws {
                collapsed.push(' ');
            }
            prev_ws = true;
        } else {
            collapsed.push(c);
            prev_ws = false;
        }
    }
    // 3. drop spaces adjacent to tag boundaries.
    collapsed
        .replace("> ", ">")
        .replace(" <", "<")
        .trim()
        .to_string()
}

fn golden_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ui/tests")
        .join(name)
}

/// Assert `actual` equals the committed golden, or (re)write it under `REGEN`.
fn assert_or_regen(name: &str, actual: &str) {
    let path = golden_path(name);
    if std::env::var("REGEN").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual).unwrap();
        eprintln!("[REGEN] wrote {}", path.display());
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing golden {name}; run `REGEN=1 cargo test`"));
    assert_eq!(actual.trim(), expected.trim(), "golden {name} drifted");
}

/// A deterministic layout for parity: the default Customers form (per field a
/// label `text` object + a value `field` object, #60), both field objects made
/// read-only (so Browse renders the display/non-editing state #44 compares),
/// Email lifted to z=5, plus a free static `text` object and a `rect` shape with
/// appearance props — covering fm-field / fm-readonly / z-index / fm-text /
/// fm-shape and the server-derived shape_style in one fixture.
fn parity_fixture() -> (Solution, i64) {
    let mut sol = Solution::open_in_memory().unwrap();
    let tid = sol
        .create_table(
            "Customers",
            &[
                NewField {
                    name: "Name".into(),
                    kind: FieldKind::Text,
                },
                NewField {
                    name: "Email".into(),
                    kind: FieldKind::Text,
                },
            ],
        )
        .unwrap();
    let table = sol.table_by_name("Customers").unwrap().unwrap();
    let fields = sol.fields(tid).unwrap();
    sol.insert_record(
        &table,
        &[
            (&fields[0], "Ada".into()),
            (&fields[1], "ada@example.com".into()),
        ],
    )
    .unwrap();
    let layout_id = sol.layouts().unwrap()[0].id;
    sol.app
        .execute(
            "UPDATE meta_object SET read_only=1 WHERE binding='Customers.Name'",
            [],
        )
        .unwrap();
    sol.app
        .execute(
            "UPDATE meta_object SET read_only=1, z=5 WHERE binding='Customers.Email'",
            [],
        )
        .unwrap();
    let part_id: i64 = sol
        .app
        .query_row(
            "SELECT id FROM meta_part WHERE layout_id=?1 AND kind='body'",
            [layout_id],
            |r| r.get(0),
        )
        .unwrap();
    sol.app
        .execute(
            "INSERT INTO meta_object(part_id, kind, x, y, w, h, z, content) \
             VALUES (?1, 'text', 16, 80, 200, 24, 0, 'Note')",
            [part_id],
        )
        .unwrap();
    // A rect shape with appearance props — drives the shape kind + the
    // server-derived shape_style through the byte-equal parity gate.
    sol.app
        .execute(
            "INSERT INTO meta_object(part_id, kind, x, y, w, h, z, props) \
             VALUES (?1, 'rect', 230, 16, 64, 64, 0, \
             '{\"fill\":\"#eef\",\"stroke\":\"#88a\",\"strokeWidth\":1,\"radius\":4}')",
            [part_id],
        )
        .unwrap();
    (sol, layout_id)
}

async fn get_body(state: AppState, uri: &str) -> (StatusCode, String) {
    use axum::http::Request;
    use tower::ServiceExt;
    let resp = app(state)
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn state_for(sol: Solution) -> AppState {
    AppState::new(sol)
}

async fn post_json(state: AppState, uri: &str, body: &str) -> StatusCode {
    post_json_body(state, uri, body).await.0
}

/// POST JSON and return both the status and the response body (for endpoints
/// that echo the created object/part back to the canvas).
async fn post_json_body(state: AppState, uri: &str, body: &str) -> (StatusCode, String) {
    use axum::http::Request;
    use tower::ServiceExt;
    let resp = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn post_form_body(state: AppState, uri: &str, body: &str) -> (StatusCode, String) {
    use axum::http::Request;
    use tower::ServiceExt;
    let resp = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn schema_table_and_field_routes_manage_metadata_and_physical_table() {
    let state = state_for(Solution::open_in_memory().unwrap());
    let body = serde_json::json!({
        "name": "Invoices",
        "notes": "Billing data",
        "fields": [
            {"name": "Number", "kind": "text"},
            {"name": "Total", "kind": "number"}
        ]
    });
    let (status, resp) = post_json_body(state.clone(), "/schema/tables", &body.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    let table: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let table_id = table["id"].as_i64().unwrap();
    let table_phys = table["phys"].as_str().unwrap().to_string();
    assert_eq!(table["notes"].as_str(), Some("Billing data"));

    let (status, fields_body) =
        get_body(state.clone(), &format!("/schema/tables/{table_id}/fields")).await;
    assert_eq!(status, StatusCode::OK, "{fields_body}");
    let fields: serde_json::Value = serde_json::from_str(&fields_body).unwrap();
    let number_id = fields[0]["id"].as_i64().unwrap();
    let total_id = fields[1]["id"].as_i64().unwrap();

    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/schema/tables/{table_id}/rename"),
        r#"{"name":"Bills"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    assert!(resp.contains(r#""name":"Bills""#));
    assert!(resp.contains(r#""notes":"Billing data""#));

    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/schema/tables/{table_id}"),
        r#"{"name":"Bills","notes":"Paid and open invoices"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    assert!(resp.contains(r#""notes":"Paid and open invoices""#));

    // The update endpoint also carries renames (the dedicated rename route
    // was dropped as a strict subset of it).
    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/schema/tables/{table_id}/fields/{number_id}"),
        &serde_json::json!({
            "name": "Invoice Number",
            "kind": "text",
            "notes": "Shown on customer forms",
            "options": {
                "validation": {
                    "required": true,
                    "unique": true
                }
            }
        })
        .to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    assert!(resp.contains(r#""name":"Invoice Number""#));
    assert!(resp.contains(r#""notes":"Shown on customer forms""#));
    let field: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(
        field["options"]["validation"]["required"].as_bool(),
        Some(true)
    );
    assert_eq!(
        field["options"]["validation"]["unique"].as_bool(),
        Some(true)
    );

    // Retype likewise goes through the update endpoint.
    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/schema/tables/{table_id}/fields/{total_id}"),
        r#"{"name":"Total","kind":"text"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    assert!(resp.contains(r#""kind":"text""#));

    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/schema/tables/{table_id}/fields/order"),
        &serde_json::json!({"fieldIds": [total_id, number_id]}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    let ordered: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(ordered[0]["id"].as_i64(), Some(total_id));

    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/schema/tables/{table_id}/fields/{number_id}/delete"),
        "{}",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resp}");

    let sol = state.sol.lock().unwrap();
    let fields = sol.fields(table_id).unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].id, total_id);
    let columns: Vec<String> = {
        let mut stmt = sol
            .data
            .prepare(&format!("PRAGMA table_info({table_phys})"))
            .unwrap();
        stmt.query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert_eq!(columns, vec!["id".to_string(), fields[0].phys.clone()]);
}

#[tokio::test]
async fn value_list_routes_crud_and_resolve_items() {
    let state = state_for(Solution::open_in_memory().unwrap());
    let create = serde_json::json!({
        "name": "Sizes",
        "source": "custom",
        "config": { "values": ["Small", "-", "Large"] }
    });
    let (status, resp) = post_json_body(state.clone(), "/value-lists", &create.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    let list: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let id = list["id"].as_i64().unwrap();
    assert_eq!(list["name"].as_str(), Some("Sizes"));
    assert_eq!(list["config"]["values"][0].as_str(), Some("Small"));

    let (status, resp) = get_body(state.clone(), "/value-lists").await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    let lists: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(lists.as_array().unwrap().len(), 1);

    let (status, resp) = get_body(state.clone(), &format!("/value-lists/{id}/items")).await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    let items: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(items[0]["value"].as_str(), Some("Small"));
    assert_eq!(items[1]["divider"].as_bool(), Some(true));
    assert_eq!(items[2]["value"].as_str(), Some("Large"));

    let update = serde_json::json!({
        "name": "Sizes Updated",
        "source": "custom",
        "config": { "values": ["Medium"] }
    });
    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/value-lists/{id}"),
        &update.to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    assert!(resp.contains("Sizes Updated"));

    let (status, resp) =
        post_json_body(state.clone(), &format!("/value-lists/{id}/duplicate"), "{}").await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    assert!(resp.contains("Sizes Updated Copy"));

    let (status, resp) =
        post_json_body(state.clone(), &format!("/value-lists/{id}/delete"), "{}").await;
    assert_eq!(status, StatusCode::OK, "{resp}");
}

#[tokio::test]
async fn record_commits_enforce_required_unique_range_and_value_list_constraints() {
    let mut sol = Solution::open_in_memory().unwrap();
    let table_id = sol
        .create_table(
            "Invoices",
            &[
                NewField {
                    name: "Number".into(),
                    kind: FieldKind::Text,
                },
                NewField {
                    name: "Total".into(),
                    kind: FieldKind::Number,
                },
                NewField {
                    name: "Status".into(),
                    kind: FieldKind::Text,
                },
            ],
        )
        .unwrap();
    let fields = sol.fields(table_id).unwrap();
    let number = fields.iter().find(|f| f.name == "Number").unwrap().clone();
    let total = fields.iter().find(|f| f.name == "Total").unwrap().clone();
    let status_field = fields.iter().find(|f| f.name == "Status").unwrap().clone();
    let statuses = sol
        .create_value_list(&NewValueList {
            name: "Statuses".into(),
            source: "custom".into(),
            config: r#"{"values":["Open","Closed"]}"#.into(),
        })
        .unwrap();
    sol.update_field_options(table_id, number.id, r#"{"validation":{"primary":true}}"#)
        .unwrap();
    sol.update_field_options(
        table_id,
        total.id,
        r#"{"validation":{"range":{"min":"1","max":"10"}}}"#,
    )
    .unwrap();
    sol.update_field_options(
        table_id,
        status_field.id,
        &format!(
            r#"{{"validation":{{"memberOfValueList":{}}}}}"#,
            statuses.id
        ),
    )
    .unwrap();
    let form_layout = sol
        .layouts_for_table(table_id)
        .unwrap()
        .into_iter()
        .find(|l| l.view == "form")
        .unwrap();
    let state = state_for(sol);

    let (status, body) = post_form_body(
        state.clone(),
        &format!("/browse/{}", form_layout.id),
        &format!("f{}=&f{}=5&f{}=Open", number.id, total.id, status_field.id),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("Number"));
    assert!(body.contains("required"));

    let (status, body) = post_form_body(
        state.clone(),
        &format!("/browse/{}", form_layout.id),
        &format!(
            "f{}=INV-1&f{}=15&f{}=Open",
            number.id, total.id, status_field.id
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("Total"));
    assert!(body.contains("at most 10"));

    let (status, body) = post_form_body(
        state.clone(),
        &format!("/browse/{}", form_layout.id),
        &format!(
            "f{}=INV-1&f{}=5&f{}=Draft",
            number.id, total.id, status_field.id
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("Status"));
    assert!(body.contains("value list"));

    let (status, _body) = post_form_body(
        state.clone(),
        &format!("/browse/{}", form_layout.id),
        &format!(
            "f{}=INV-1&f{}=5&f{}=Open%0AClosed",
            number.id, total.id, status_field.id
        ),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);

    let (status, body) = post_form_body(
        state,
        &format!("/browse/{}", form_layout.id),
        &format!(
            "f{}=INV-1&f{}=6&f{}=Closed",
            number.id, total.id, status_field.id
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body.contains("Number"));
    assert!(body.contains("unique"));
}

#[tokio::test]
async fn field_reference_options_sync_relationship_edges() {
    let mut sol = Solution::open_in_memory().unwrap();
    let customers = sol
        .create_table(
            "Customers",
            &[NewField {
                name: "Id".into(),
                kind: FieldKind::Number,
            }],
        )
        .unwrap();
    let invoices = sol
        .create_table(
            "Invoices",
            &[NewField {
                name: "Customer Id".into(),
                kind: FieldKind::Number,
            }],
        )
        .unwrap();
    let customer_id = sol.fields(customers).unwrap()[0].id;
    let invoice_customer_id = sol.fields(invoices).unwrap()[0].id;
    let state = state_for(sol);

    let body = serde_json::json!({
        "name": "Customer Id",
        "kind": "number",
        "options": {
            "reference": {
                "name": "customer",
                "toTable": customers,
                "toField": customer_id
            }
        }
    });
    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/schema/tables/{invoices}/fields/{invoice_customer_id}"),
        &body.to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    let field: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(
        field["options"]["reference"]["name"].as_str(),
        Some("customer")
    );

    let (status, resp) = get_body(state.clone(), "/schema/relationships").await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    let relationships: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(relationships.as_array().unwrap().len(), 1);
    assert_eq!(relationships[0]["name"].as_str(), Some("customer"));
    assert_eq!(
        relationships[0]["fromField"].as_i64(),
        Some(invoice_customer_id)
    );

    let body = serde_json::json!({
        "name": "Customer Id",
        "kind": "number",
        "options": {}
    });
    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/schema/tables/{invoices}/fields/{invoice_customer_id}"),
        &body.to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    let (status, resp) = get_body(state, "/schema/relationships").await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    let relationships: serde_json::Value = serde_json::from_str(&resp).unwrap();
    assert!(relationships.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn schema_relationship_routes_crud_and_validate_field_ownership() {
    let mut sol = Solution::open_in_memory().unwrap();
    let customers = sol
        .create_table(
            "Customers",
            &[NewField {
                name: "Id".into(),
                kind: FieldKind::Number,
            }],
        )
        .unwrap();
    let invoices = sol
        .create_table(
            "Invoices",
            &[NewField {
                name: "Customer Id".into(),
                kind: FieldKind::Number,
            }],
        )
        .unwrap();
    let customer_id = sol.fields(customers).unwrap()[0].id;
    let invoice_customer_id = sol.fields(invoices).unwrap()[0].id;
    let state = state_for(sol);

    let bad = serde_json::json!({
        "name": "bad",
        "fromTable": invoices,
        "toTable": customers,
        "fromField": customer_id,
        "toField": invoice_customer_id
    });
    let (status, _) =
        post_json_body(state.clone(), "/schema/relationships", &bad.to_string()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let create = serde_json::json!({
        "name": "customer",
        "fromTable": invoices,
        "toTable": customers,
        "fromField": invoice_customer_id,
        "toField": customer_id
    });
    let (status, resp) =
        post_json_body(state.clone(), "/schema/relationships", &create.to_string()).await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    let rel: serde_json::Value = serde_json::from_str(&resp).unwrap();
    let rel_id = rel["id"].as_i64().unwrap();
    let (_, fields_body) =
        get_body(state.clone(), &format!("/schema/tables/{invoices}/fields")).await;
    let fields: serde_json::Value = serde_json::from_str(&fields_body).unwrap();
    assert_eq!(
        fields[0]["options"]["reference"]["name"].as_str(),
        Some("customer")
    );

    let update = serde_json::json!({
        "name": "bill_to",
        "fromTable": invoices,
        "toTable": customers,
        "fromField": invoice_customer_id,
        "toField": customer_id
    });
    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/schema/relationships/{rel_id}"),
        &update.to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    assert!(resp.contains(r#""name":"bill_to""#));
    let (_, fields_body) =
        get_body(state.clone(), &format!("/schema/tables/{invoices}/fields")).await;
    let fields: serde_json::Value = serde_json::from_str(&fields_body).unwrap();
    assert_eq!(
        fields[0]["options"]["reference"]["name"].as_str(),
        Some("bill_to")
    );

    let (status, resp) = get_body(state.clone(), "/schema/relationships").await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    assert!(resp.contains(r#""fromTable":"#));

    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/schema/relationships/{rel_id}/delete"),
        "{}",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    let (_, fields_body) = get_body(state, &format!("/schema/tables/{invoices}/fields")).await;
    let fields: serde_json::Value = serde_json::from_str(&fields_body).unwrap();
    assert!(fields[0]["options"].get("reference").is_none());
}

/// #57: a table carries independent per-view layouts. The Browse view toggle
/// links to sibling layout ids (not one layout re-rendered via `?view=`), and
/// each layout renders in its own view.
#[tokio::test]
async fn browse_view_tabs_link_to_sibling_layouts_and_render_by_view() {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[NewField {
            name: "Name".into(),
            kind: FieldKind::Text,
        }],
    )
    .unwrap();
    let table = sol.table_by_name("Customers").unwrap().unwrap();
    let fields = sol.fields(table.id).unwrap();
    sol.insert_record(&table, &[(&fields[0], "Ada".into())])
        .unwrap();
    let layouts = sol.layouts_for_table(table.id).unwrap();
    let form = layouts.iter().find(|l| l.view == "form").unwrap().id;
    let list = layouts.iter().find(|l| l.view == "list").unwrap().id;
    let table_l = layouts.iter().find(|l| l.view == "table").unwrap().id;
    assert!(
        form != list && list != table_l && form != table_l,
        "distinct per-view ids"
    );
    let state = state_for(sol);

    // The Form layout renders the canvas and offers tabs to the SIBLING ids.
    let (status, html) = get_body(state.clone(), &format!("/browse/{form}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains(r#"<div class="fm-canvas""#),
        "form renders the canvas"
    );
    assert!(
        html.contains(&format!(r#"href="/browse/{list}""#)),
        "List tab → list layout"
    );
    assert!(
        html.contains(&format!(r#"href="/browse/{table_l}""#)),
        "Table tab → table layout"
    );

    // The List layout renders the list surface by its own view, not the canvas.
    let (status, html) = get_body(state, &format!("/browse/{list}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains(r#"class="fm-list""#),
        "list renders the list surface"
    );
    assert!(
        !html.contains(r#"<div class="fm-canvas""#),
        "list view is not the form canvas"
    );
}

/// Table Browse frames its field-grid with the layout's header/footer bands,
/// the same as Form/List — so all three views share the fixed-band shape.
#[tokio::test]
async fn table_view_renders_header_and_footer_bands() {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[NewField {
            name: "Name".into(),
            kind: FieldKind::Text,
        }],
    )
    .unwrap();
    let table = sol.table_by_name("Customers").unwrap().unwrap();
    sol.insert_record(&table, &[]).unwrap();
    let table_l = sol
        .layouts_for_table(table.id)
        .unwrap()
        .into_iter()
        .find(|l| l.view == "table")
        .unwrap()
        .id;

    let (status, html) = get_body(state_for(sol), &format!("/browse/{table_l}")).await;
    assert_eq!(status, StatusCode::OK);
    // Still the field-derived grid…
    assert!(html.contains(r#"class="fm-tableview""#) && html.contains("<thead>"));
    // …now wrapped by header/footer band regions.
    assert!(
        html.contains(r#"<div class="fm-bands-head">"#),
        "table view renders the header band region"
    );
    assert!(
        html.contains(r#"<div class="fm-bands-foot">"#),
        "table view renders the footer band region"
    );
    // The layout's header + footer parts both render as bands (the grid body
    // is field-derived, so these are the only .fm-part divs in Table view).
    assert!(
        html.matches(r#"class="fm-part""#).count() >= 2,
        "both header and footer bands render their parts"
    );
}

/// #57 Layout-mode chrome: the view toggle stays (switching which view you
/// DESIGN, via /design/ siblings) and the pagination control is repurposed to
/// step layouts; record actions are Browse-only.
#[tokio::test]
async fn design_mode_keeps_view_toggle_and_layout_stepper() {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[NewField {
            name: "Name".into(),
            kind: FieldKind::Text,
        }],
    )
    .unwrap();
    let table = sol.table_by_name("Customers").unwrap().unwrap();
    let layouts = sol.layouts_for_table(table.id).unwrap();
    let form = layouts.iter().find(|l| l.view == "form").unwrap().id;
    let list = layouts.iter().find(|l| l.view == "list").unwrap().id;
    let (status, html) = get_body(state_for(sol), &format!("/design/{form}")).await;
    assert_eq!(status, StatusCode::OK);
    // View toggle present, switching which view you DESIGN (links into /design/).
    assert!(
        html.contains(&format!(r#"href="/design/{list}""#)),
        "view toggle → design the List layout"
    );
    // Pagination control repurposed to layout navigation.
    assert!(
        html.contains("Layout navigation"),
        "stepper navigates layouts in design mode"
    );
    // Record actions don't belong in Layout mode.
    assert!(
        html.contains(r#"title="Records are managed in Browse mode""#),
        "no record actions in layout mode"
    );
}

/// #46 group commit: a bulk POST persists every object's geometry in one
/// request (scoped + clamped), returns the updated count, and skips unknown ids.
#[tokio::test]
async fn design_bulk_geometry_persists_group() {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[
            NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            },
            NewField {
                name: "Email".into(),
                kind: FieldKind::Text,
            },
        ],
    )
    .unwrap();
    let layout_id = sol.layouts().unwrap()[0].id;
    let part = body_part(&sol, layout_id);
    let objs = sol.objects(part.id).unwrap();
    let (a, b) = (objs[0].id, objs[1].id);
    let state = state_for(sol);

    let resp = {
        use axum::http::Request;
        use tower::ServiceExt;
        let body = format!(
            r#"[{{"id":{a},"x":10,"y":20,"w":100,"h":24}},{{"id":{b},"x":-5,"y":40,"w":100,"h":24}},{{"id":999999,"x":0,"y":0,"w":1,"h":1}}]"#
        );
        app(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/design/{layout_id}/geometry"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap()
    };
    assert_eq!(resp.status(), StatusCode::OK);
    let count = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        String::from_utf8(count.to_vec()).unwrap(),
        "2",
        "only real ids count"
    );

    let sol = state.sol.lock().unwrap();
    let after = sol.objects(part.id).unwrap();
    assert_eq!((after[0].x, after[0].y), (10, 20));
    assert_eq!(
        (after[1].x, after[1].y),
        (0, 40),
        "negative x clamped to origin"
    );
}

/// #83 z-order: a bulk POST to `/z` persists every object's stacking order in
/// one request (scoped), returns the updated count, and skips unknown ids.
#[tokio::test]
async fn design_bulk_z_persists_group() {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[
            NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            },
            NewField {
                name: "Email".into(),
                kind: FieldKind::Text,
            },
        ],
    )
    .unwrap();
    let layout_id = sol.layouts().unwrap()[0].id;
    let part = body_part(&sol, layout_id);
    let objs = sol.objects(part.id).unwrap();
    let (a, b) = (objs[0].id, objs[1].id);
    let state = state_for(sol);

    let resp = {
        use axum::http::Request;
        use tower::ServiceExt;
        let body = format!(r#"[{{"id":{a},"z":3}},{{"id":{b},"z":7}},{{"id":999999,"z":1}}]"#);
        app(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/design/{layout_id}/z"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap()
    };
    assert_eq!(resp.status(), StatusCode::OK);
    let count = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        String::from_utf8(count.to_vec()).unwrap(),
        "2",
        "only real ids count"
    );

    let sol = state.sol.lock().unwrap();
    let after = sol.objects(part.id).unwrap();
    // `objects()` sorts by (z, id), so read back by id rather than position.
    assert_eq!(after.iter().find(|o| o.id == a).unwrap().z, 3);
    assert_eq!(after.iter().find(|o| o.id == b).unwrap().z, 7);
}

/// #75 durable groups: the group relationship persists in the design model,
/// and Ungroup removes only the relationship, not child geometry/styles.
#[tokio::test]
async fn design_object_group_persists_and_ungroups() {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[
            NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            },
            NewField {
                name: "Email".into(),
                kind: FieldKind::Text,
            },
        ],
    )
    .unwrap();
    let layout_id = sol.layouts().unwrap()[0].id;
    let part = body_part(&sol, layout_id);
    let objs = sol.objects(part.id).unwrap();
    let (a, b) = (objs[0].id, objs[1].id);
    let state = state_for(sol);

    let (status, body) = post_json_body(
        state.clone(),
        &format!("/design/{layout_id}/group"),
        &format!(r#"{{"objectIds":[{a},{b}]}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(&format!(r#""objectIds":[{a},{b}]"#)),
        "{body}"
    );

    let (_, model) = get_body(state.clone(), &format!("/design/{layout_id}/model")).await;
    assert!(
        model.contains(&format!(r#""groups":[{{"id":1,"objectIds":[{a},{b}]}}]"#)),
        "model includes durable group\n{model}"
    );

    let status = post_json(
        state.clone(),
        &format!("/design/{layout_id}/group/1/delete"),
        "{}",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, model) = get_body(state.clone(), &format!("/design/{layout_id}/model")).await;
    assert!(model.contains(r#""groups":[]"#), "group removed\n{model}");
    assert!(
        model.contains(&format!(r#""id":{a}"#)) && model.contains(&format!(r#""id":{b}"#)),
        "ungroup leaves child objects in place\n{model}"
    );

    let (status, body) = post_json_body(
        state.clone(),
        &format!("/design/{layout_id}/group"),
        &format!(r#"{{"id":42,"objectIds":[{a},{b}]}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(&format!(r#""id":42,"objectIds":[{a},{b}]"#)),
        "explicit-id group restore echoes the restored id\n{body}"
    );
    let (_, model) = get_body(state, &format!("/design/{layout_id}/model")).await;
    assert!(
        model.contains(&format!(r#""groups":[{{"id":42,"objectIds":[{a},{b}]}}]"#)),
        "model preserves restored group id\n{model}"
    );
}

/// #62 two-mount rail: the design page renders the `#layout-tools` mount node
/// in the sidebar (where the Svelte Create/Style/Zoom zones mount, sharing the
/// canvas store); Browse mode does not.
#[tokio::test]
async fn design_page_renders_tool_rail_mount_node() {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[NewField {
            name: "Name".into(),
            kind: FieldKind::Text,
        }],
    )
    .unwrap();
    let form = sol
        .layouts()
        .unwrap()
        .into_iter()
        .find(|l| l.view == "form")
        .unwrap()
        .id;
    let state = state_for(sol);

    let (status, html) = get_body(state.clone(), &format!("/design/{form}")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains(r#"id="layout-tools""#),
        "design page mounts the tool rail"
    );

    let (_, browse) = get_body(state, &format!("/browse/{form}")).await;
    assert!(
        !browse.contains(r#"id="layout-tools""#),
        "browse has no tool rail"
    );
}

/// #113: the schema-builder surface renders in `schema` mode with the single
/// island mount node and the global Schema nav marked active. It's app-global,
/// so it renders even with no tables/layouts.
#[tokio::test]
async fn schema_page_renders_builder_mount_node() {
    let sol = Solution::open_in_memory().unwrap();
    let state = state_for(sol);

    let (status, html) = get_body(state.clone(), "/schema").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains(r#"id="schema-root""#),
        "schema page mounts the builder island"
    );
    assert!(
        html.contains(r#"src="/ui/schema-builder.js""#),
        "schema page loads the schema-builder bundle"
    );

    // The builder node never appears on other surfaces.
    let (_, browse) = get_body(state, "/").await;
    assert!(
        !browse.contains(r#"id="schema-root""#),
        "the schema island is scoped to /schema"
    );
}

/// #48 create: placing a shape POSTs `{partId,kind,x,y,w,h,props}`, persists a
/// `meta_object`, and echoes back its `ObjectView` (with the server-derived
/// shape_style) so the store can add it without a re-hydrate.
#[tokio::test]
async fn design_create_shape_object_persists_and_returns_view() {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[NewField {
            name: "Name".into(),
            kind: FieldKind::Text,
        }],
    )
    .unwrap();
    let layout_id = sol.layouts().unwrap()[0].id;
    let part_id = body_part(&sol, layout_id).id;
    let before = sol.objects(part_id).unwrap().len();
    let state = state_for(sol);

    let body = format!(
        r##"{{"partId":{part_id},"kind":"rect","x":20,"y":12,"w":64,"h":48,"props":{{"fill":"#eef","stroke":"#88a","strokeWidth":1}}}}"##
    );
    let (status, resp) =
        post_json_body(state.clone(), &format!("/design/{layout_id}/object"), &body).await;
    assert_eq!(status, StatusCode::OK);
    assert!(resp.contains(r#""kind":"rect""#) && resp.contains(r#""shape":true"#));
    assert!(
        resp.contains(r#""shapeStyle":"background:#eef;box-shadow:0 0 0 1px #88a;""#),
        "derived style echoed\n{resp}"
    );
    assert!(
        resp.contains("strokeWidth"),
        "raw props echoed for the inspector\n{resp}"
    );

    let sol = state.sol.lock().unwrap();
    let objs = sol.objects(part_id).unwrap();
    assert_eq!(objs.len(), before + 1, "one row inserted");
    assert!(objs
        .iter()
        .any(|o| o.kind == ObjectKind::Rect && (o.x, o.y) == (20, 12)));
}

/// #48/#60 create: the Field tool POSTs `{kind:"field",fieldId,…}` and gets
/// back TWO views — the value field (live value resolved for the record) and
/// its spawned caption label.
#[tokio::test]
async fn design_create_field_object_spawns_label_and_returns_both() {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[NewField {
            name: "Name".into(),
            kind: FieldKind::Text,
        }],
    )
    .unwrap();
    let table = sol.table_by_name("Customers").unwrap().unwrap();
    let fields = sol.fields(table.id).unwrap();
    let name_fid = fields[0].id;
    sol.insert_record(&table, &[(&fields[0], "Ada".into())])
        .unwrap();
    let layout_id = sol.layouts().unwrap()[0].id;
    let part_id = body_part(&sol, layout_id).id;
    let before = sol.objects(part_id).unwrap().len();
    let state = state_for(sol);

    let body = format!(
        r#"{{"partId":{part_id},"kind":"field","x":120,"y":40,"w":200,"h":24,"fieldId":{name_fid},"rec":1}}"#
    );
    let (status, resp) =
        post_json_body(state.clone(), &format!("/design/{layout_id}/object"), &body).await;
    assert_eq!(status, StatusCode::OK);
    // The value field resolves "Ada" and binds Customers.Name; the label
    // carries the caption "Name".
    assert!(resp.contains(r#""kind":"field""#) && resp.contains(r#""value":"Ada""#));
    assert!(resp.contains(r#""binding":"Customers.Name""#));
    assert!(
        resp.contains(r#""kind":"text""#) && resp.contains(r#""content":"Name""#),
        "label spawned\n{resp}"
    );

    let sol = state.sol.lock().unwrap();
    assert_eq!(
        sol.objects(part_id).unwrap().len(),
        before + 2,
        "value + label inserted"
    );
    drop(sol);

    let body = format!(
        r#"{{"partId":{part_id},"kind":"field","x":120,"y":80,"w":200,"h":24,"fieldId":{name_fid},"createLabel":false,"rec":1}}"#
    );
    let (status, resp) =
        post_json_body(state.clone(), &format!("/design/{layout_id}/object"), &body).await;
    assert_eq!(status, StatusCode::OK);
    assert!(resp.contains(r#""kind":"field""#), "field created\n{resp}");
    assert!(
        !resp.contains(r#""kind":"text""#),
        "label suppressed\n{resp}"
    );

    let sol = state.sol.lock().unwrap();
    assert_eq!(
        sol.objects(part_id).unwrap().len(),
        before + 3,
        "second placement inserted value only"
    );
}

/// #85 paste: a value-only field create (createLabel:false) honors `props` so a
/// pasted field keeps its appearance. Regression for the value-only branch
/// silently dropping props — the derived shape style must round-trip + persist.
#[tokio::test]
async fn design_field_paste_create_honors_props() {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[NewField {
            name: "Name".into(),
            kind: FieldKind::Text,
        }],
    )
    .unwrap();
    let table = sol.table_by_name("Customers").unwrap().unwrap();
    let name_fid = sol.fields(table.id).unwrap()[0].id;
    let layout_id = sol.layouts().unwrap()[0].id;
    let part_id = body_part(&sol, layout_id).id;
    let state = state_for(sol);

    let body = format!(
        r##"{{"partId":{part_id},"kind":"field","x":10,"y":10,"w":120,"h":24,"fieldId":{name_fid},"createLabel":false,"rec":1,"props":{{"fill":"#ffeecc","stroke":"#335577","strokeWidth":3}}}}"##
    );
    let (status, resp) =
        post_json_body(state.clone(), &format!("/design/{layout_id}/object"), &body).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        resp.contains(r#""objectStyle":"background:#ffeecc;box-shadow:0 0 0 3px #335577;""#),
        "pasted field keeps props-derived style\n{resp}"
    );
    let (_, model) = get_body(state, &format!("/design/{layout_id}/model")).await;
    assert!(
        model.contains(r#""objectStyle":"background:#ffeecc;box-shadow:0 0 0 3px #335577;""#),
        "pasted field props persist in the model\n{model}"
    );
}

/// #48 duplicate: a value-only field copy (createLabel:false) carries the
/// source object's `binding` verbatim, so Ctrl/Cmd+D round-trips even when the
/// binding doesn't resolve to a live field_id — an empty table (no records)
/// renders every field object with `field_id: null`, exactly the state that
/// used to 400 "field tool needs a fieldId".
#[tokio::test]
async fn design_duplicate_field_by_binding_without_field_id() {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[NewField {
            name: "Name".into(),
            kind: FieldKind::Text,
        }],
    )
    .unwrap();
    // No records inserted: with an empty table the read model resolves no
    // value, so a field object's field_id is null — the crashing scenario.
    let layout_id = sol.layouts().unwrap()[0].id;
    let part_id = body_part(&sol, layout_id).id;
    let before = sol.objects(part_id).unwrap().len();
    let state = state_for(sol);

    // Exactly what the canvas POSTs on Ctrl/Cmd+D of a field whose field_id is
    // null: no fieldId, but the binding fully determines the copy.
    let body = format!(
        r#"{{"partId":{part_id},"kind":"field","x":40,"y":40,"w":120,"h":24,"fieldId":null,"createLabel":false,"binding":"Customers.Name"}}"#
    );
    let (status, resp) =
        post_json_body(state.clone(), &format!("/design/{layout_id}/object"), &body).await;
    assert_eq!(status, StatusCode::OK, "duplicate by binding\n{resp}");
    assert!(resp.contains(r#""kind":"field""#), "field created\n{resp}");
    assert!(
        !resp.contains(r#""kind":"text""#),
        "no caption spawned for a value-only copy\n{resp}"
    );

    let sol = state.sol.lock().unwrap();
    let objs = sol.objects(part_id).unwrap();
    assert_eq!(objs.len(), before + 1, "one value-only row inserted");
    let created = objs.iter().find(|o| (o.x, o.y) == (40, 40)).unwrap();
    assert_eq!(created.kind, ObjectKind::Field);
    assert_eq!(
        created.binding.as_deref(),
        Some("Customers.Name"),
        "source binding preserved verbatim"
    );
}

#[tokio::test]
async fn design_selected_object_inspector_updates_field_text_and_read_only() {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[
            NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            },
            NewField {
                name: "Email".into(),
                kind: FieldKind::Text,
            },
        ],
    )
    .unwrap();
    let table = sol.table_by_name("Customers").unwrap().unwrap();
    let fields = sol.fields(table.id).unwrap();
    sol.insert_record(
        &table,
        &[
            (&fields[0], "Ada".into()),
            (&fields[1], "ada@example.test".into()),
        ],
    )
    .unwrap();
    let layout_id = sol.layouts().unwrap()[0].id;
    let part_id = body_part(&sol, layout_id).id;
    let objects = sol.objects(part_id).unwrap();
    let label_id = objects
        .iter()
        .find(|o| o.kind == ObjectKind::Text)
        .unwrap()
        .id;
    let field_id = objects
        .iter()
        .find(|o| o.kind == ObjectKind::Field)
        .unwrap()
        .id;
    let email_fid = fields[1].id;
    let state = state_for(sol);

    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/design/{layout_id}/object/{field_id}/binding"),
        &format!(r#"{{"fieldId":{email_fid},"rec":1}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        resp.contains(r#""binding":"Customers.Email""#),
        "binding response\n{resp}"
    );
    assert!(
        resp.contains(r#""fieldId":"#) && resp.contains(r#""value":"ada@example.test""#),
        "field projection\n{resp}"
    );

    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/design/{layout_id}/object/{label_id}/content"),
        r#"{"content":"Primary email"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        resp.contains(r#""content":"Primary email""#),
        "content response\n{resp}"
    );

    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/design/{layout_id}/object/{field_id}/read-only"),
        r#"{"readOnly":true,"rec":1}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        resp.contains(r#""readOnly":true"#),
        "read-only response\n{resp}"
    );

    let sol = state.sol.lock().unwrap();
    let updated = sol.objects(part_id).unwrap();
    let label = updated.iter().find(|o| o.id == label_id).unwrap();
    let field = updated.iter().find(|o| o.id == field_id).unwrap();
    assert_eq!(label.content.as_deref(), Some("Primary email"));
    assert_eq!(field.binding.as_deref(), Some("Customers.Email"));
    assert!(field.read_only);
}

#[tokio::test]
async fn design_object_props_style_field_and_text_objects() {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[NewField {
            name: "Name".into(),
            kind: FieldKind::Text,
        }],
    )
    .unwrap();
    let layout_id = sol.layouts().unwrap()[0].id;
    let part_id = body_part(&sol, layout_id).id;
    let objects = sol.objects(part_id).unwrap();
    let label_id = objects
        .iter()
        .find(|o| o.kind == ObjectKind::Text)
        .unwrap()
        .id;
    let field_id = objects
        .iter()
        .find(|o| o.kind == ObjectKind::Field)
        .unwrap()
        .id;
    let state = state_for(sol);

    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/design/{layout_id}/object/{field_id}/props"),
        r##"{"props":{"fill":"#ffeecc","stroke":"#335577","strokeWidth":3,"textColor":"#112233","fontSize":18,"bold":true,"italic":true,"underline":true,"align":"right"}}"##,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        resp.contains(r#""objectStyle":"background:#ffeecc;box-shadow:0 0 0 3px #335577;""#),
        "field box style\n{resp}"
    );
    assert!(
        resp.contains("color:#112233;font-size:18px;font-weight:700;font-style:italic;text-decoration:underline;text-align:right;justify-content:flex-end;"),
        "field text style\n{resp}"
    );

    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/design/{layout_id}/object/{label_id}/props"),
        r##"{"props":{"textColor":"#445566","fontSize":16,"align":"center"}}"##,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        resp.contains(r#""textStyle":"color:#445566;font-size:16px;text-align:center;justify-content:center;""#),
        "text formatting style\n{resp}"
    );

    let (_, model) = get_body(state, &format!("/design/{layout_id}/model")).await;
    assert!(
        model.contains(r#""objectStyle":"background:#ffeecc;box-shadow:0 0 0 3px #335577;""#)
            && model.contains(r#""textStyle":"color:#445566;font-size:16px;text-align:center;justify-content:center;""#),
        "styles persist in design model\n{model}"
    );
}

/// #48 create-part: POSTing a kind appends a band and echoes its `PartView`.
#[tokio::test]
async fn design_create_part_appends_band_and_returns_view() {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[NewField {
            name: "Name".into(),
            kind: FieldKind::Text,
        }],
    )
    .unwrap();
    // Summaries are a List/Table feature (Issue 3): design on the List view.
    let layout_id = sol
        .layouts()
        .unwrap()
        .into_iter()
        .find(|l| l.view == "list")
        .unwrap()
        .id;
    let before = sol.parts(layout_id).unwrap().len();
    let state = state_for(sol);

    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/design/{layout_id}/part"),
        r#"{"kind":"subsummary","height":40}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(resp.contains(r#""kind":"subsummary""#) && resp.contains(r#""height":40"#));
    // The response carries the post-insert ordering so the client resyncs
    // positions instead of guessing bottom-most.
    assert!(
        resp.contains(r#""positions""#),
        "create echoes positions\n{resp}"
    );
    let parts = state.sol.lock().unwrap().parts(layout_id).unwrap();
    assert_eq!(parts.len(), before + 1);
    // The new summary must sit ABOVE the footer — never below it.
    let sub = parts
        .iter()
        .find(|p| p.kind == PartKind::SubSummary)
        .unwrap();
    let footer = parts.iter().find(|p| p.kind == PartKind::Footer).unwrap();
    assert!(
        sub.position < footer.position,
        "sub-summary must land above the footer (sub {} vs footer {})",
        sub.position,
        footer.position
    );
}

/// Part editing: height/kind/delete round-trip through layout-scoped design
/// endpoints, and deleting a band removes its child objects.
#[tokio::test]
async fn design_part_editing_round_trip() {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[NewField {
            name: "Name".into(),
            kind: FieldKind::Text,
        }],
    )
    .unwrap();
    // Summaries are a List/Table feature (Issue 3): design on the List view.
    let layout_id = sol
        .layouts()
        .unwrap()
        .into_iter()
        .find(|l| l.view == "list")
        .unwrap()
        .id;
    let part_id = sol
        .create_part(layout_id, PartKind::SubSummary, 80)
        .unwrap();
    let state = state_for(sol);

    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/design/{layout_id}/part/{part_id}/height"),
        r#"{"height":164}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(resp.contains(r#""height":164"#));
    assert_eq!(
        state
            .sol
            .lock()
            .unwrap()
            .part_by_id(layout_id, part_id)
            .unwrap()
            .unwrap()
            .height,
        164
    );

    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/design/{layout_id}/part/{part_id}/kind"),
        r#"{"kind":"grandsummary"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(resp.contains(r#""kind":"grandsummary""#));
    assert_eq!(
        state
            .sol
            .lock()
            .unwrap()
            .part_by_id(layout_id, part_id)
            .unwrap()
            .unwrap()
            .kind,
        PartKind::GrandSummary
    );
    let body_id = state
        .sol
        .lock()
        .unwrap()
        .parts(layout_id)
        .unwrap()
        .into_iter()
        .find(|p| p.kind == PartKind::Body)
        .unwrap()
        .id;
    assert_eq!(
        post_json(
            state.clone(),
            &format!("/design/{layout_id}/part/{body_id}/kind"),
            r#"{"kind":"header"}"#
        )
        .await,
        StatusCode::CONFLICT
    );

    assert_eq!(
        post_json(
            state.clone(),
            &format!("/design/{}/part/{part_id}/height", layout_id + 999),
            r#"{"height":1}"#
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post_json(
            state.clone(),
            &format!("/design/{layout_id}/part/{part_id}/kind"),
            r#"{"kind":"bad"}"#
        )
        .await,
        StatusCode::BAD_REQUEST
    );

    assert_eq!(
        post_json(
            state.clone(),
            &format!("/design/{layout_id}/part/{body_id}/delete"),
            ""
        )
        .await,
        StatusCode::CONFLICT
    );
    assert_eq!(
        post_json(
            state.clone(),
            &format!("/design/{layout_id}/part/{part_id}/delete"),
            ""
        )
        .await,
        StatusCode::OK
    );
    let sol = state.sol.lock().unwrap();
    assert!(sol.part_by_id(layout_id, part_id).unwrap().is_none());
    assert!(
        sol.objects(part_id).unwrap().is_empty(),
        "objects deleted with the band"
    );
}

/// Issue 7: setting a band's fill persists its `props`, echoes the re-derived
/// `part_style`, and surfaces on the next model/Browse read; a foreign layout
/// id is a scoped no-op (404).
#[tokio::test]
async fn design_part_props_sets_band_fill_and_is_scoped() {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[NewField {
            name: "Name".into(),
            kind: FieldKind::Text,
        }],
    )
    .unwrap();
    let layout_id = sol.layouts().unwrap()[0].id;
    let part_id = body_part(&sol, layout_id).id;
    let state = state_for(sol);

    // A fill commit echoes the raw props AND the server-derived part_style.
    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/design/{layout_id}/part/{part_id}/props"),
        r##"{"props":{"fill":"#334455"}}"##,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        resp.contains(r#""partStyle":"background:#334455;""#),
        "derived band style echoed\n{resp}"
    );
    assert_eq!(
        state
            .sol
            .lock()
            .unwrap()
            .part_by_id(layout_id, part_id)
            .unwrap()
            .unwrap()
            .props
            .as_deref(),
        Some(r##"{"fill":"#334455"}"##)
    );

    // The design model carries the derived style so the canvas renders it live.
    let (_, model) = get_body(state.clone(), &format!("/design/{layout_id}/model")).await;
    assert!(
        model.contains(r#""partStyle":"background:#334455;""#),
        "band fill persists in design model\n{model}"
    );

    // A foreign layout id is a scoped no-op ⇒ 404.
    assert_eq!(
        post_json(
            state.clone(),
            &format!("/design/{}/part/{part_id}/props", layout_id + 999),
            r##"{"props":{"fill":"#000000"}}"##,
        )
        .await,
        StatusCode::NOT_FOUND
    );
}

/// Issue 4: the move endpoint reorders a summary band and returns the layout's
/// `[{id, position}]` after the move; a clamped move (past the footer) is 404.
#[tokio::test]
async fn design_move_part_reorders_and_returns_positions() {
    let mut sol = Solution::open_in_memory().unwrap();
    let tid = sol
        .create_table(
            "Customers",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
    // Summaries live on List/Table (Issue 3).
    let layout_id = sol
        .layouts_for_table(tid)
        .unwrap()
        .into_iter()
        .find(|l| l.view == "list")
        .unwrap()
        .id;
    // header, body, sub, grand, footer.
    let sub = sol
        .create_part(layout_id, PartKind::SubSummary, 40)
        .unwrap();
    let grand = sol
        .create_part(layout_id, PartKind::GrandSummary, 40)
        .unwrap();
    let state = state_for(sol);

    // Move the grand summary up: it swaps with the sub summary; response lists
    // every part's post-move position.
    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/design/{layout_id}/part/{grand}/move"),
        r#"{"up":true}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(resp.contains(&format!(r#"{{"id":{grand},"position":2}}"#)));
    assert!(resp.contains(&format!(r#"{{"id":{sub},"position":3}}"#)));

    // The sub summary can't move below the footer — clamped ⇒ 404.
    assert_eq!(
        post_json(
            state.clone(),
            &format!("/design/{layout_id}/part/{sub}/move"),
            r#"{"up":false}"#
        )
        .await,
        StatusCode::NOT_FOUND
    );
}

/// #48 delete + #49 props: a placed object can have its props set (shape style
/// re-derives on the next read) and can be deleted; both are layout-scoped.
#[tokio::test]
async fn design_object_props_then_delete_round_trip() {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[NewField {
            name: "Name".into(),
            kind: FieldKind::Text,
        }],
    )
    .unwrap();
    let layout_id = sol.layouts().unwrap()[0].id;
    let part_id = body_part(&sol, layout_id).id;
    let state = state_for(sol);

    // Create a rect to operate on.
    let (status, _) = post_json_body(
        state.clone(),
        &format!("/design/{layout_id}/object"),
        &format!(r#"{{"partId":{part_id},"kind":"rect","x":0,"y":0,"w":40,"h":40}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rect_id = {
        let sol = state.sol.lock().unwrap();
        sol.objects(part_id)
            .unwrap()
            .iter()
            .find(|o| o.kind == ObjectKind::Rect)
            .unwrap()
            .id
    };

    // Set props → the model now derives a shape_style from them.
    let status = post_json(
        state.clone(),
        &format!("/design/{layout_id}/object/{rect_id}/props"),
        r##"{"props":{"fill":"#102030","radius":6}}"##,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, model) = get_body(state.clone(), &format!("/design/{layout_id}/model")).await;
    assert!(
        model.contains("background:#102030;border-radius:6px;"),
        "props drive shape_style\n{model}"
    );

    // Delete it (scoped): a foreign layout is a no-op 404, the real one 200.
    assert_eq!(
        post_json(
            state.clone(),
            &format!("/design/{}/object/{rect_id}/delete", layout_id + 999),
            ""
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post_json(
            state.clone(),
            &format!("/design/{layout_id}/object/{rect_id}/delete"),
            ""
        )
        .await,
        StatusCode::OK
    );
    assert!(!state
        .sol
        .lock()
        .unwrap()
        .objects(part_id)
        .unwrap()
        .iter()
        .any(|o| o.id == rect_id));
}

/// #84 restore: helper that creates a rect and returns (state, layout_id,
/// part_id, rect_id) ready for a delete→restore round-trip.
async fn seeded_rect() -> (AppState, i64, i64, i64) {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[NewField {
            name: "Name".into(),
            kind: FieldKind::Text,
        }],
    )
    .unwrap();
    let layout_id = sol.layouts().unwrap()[0].id;
    let part_id = body_part(&sol, layout_id).id;
    let state = state_for(sol);
    let (status, _) = post_json_body(
        state.clone(),
        &format!("/design/{layout_id}/object"),
        &format!(r#"{{"partId":{part_id},"kind":"rect","x":7,"y":9,"w":40,"h":40}}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rect_id = {
        let sol = state.sol.lock().unwrap();
        sol.objects(part_id)
            .unwrap()
            .iter()
            .find(|o| o.kind == ObjectKind::Rect)
            .unwrap()
            .id
    };
    (state, layout_id, part_id, rect_id)
}

fn object_ids(state: &AppState, part_id: i64) -> Vec<i64> {
    state
        .sol
        .lock()
        .unwrap()
        .objects(part_id)
        .unwrap()
        .iter()
        .map(|o| o.id)
        .collect()
}

/// #84 undo-of-delete: restore re-inserts a deleted object at its EXACT
/// original id (identity preserved so bindings/labels survive), with its
/// geometry and props intact and visible on the next model read.
#[tokio::test]
async fn design_object_restore_preserves_identity() {
    let (state, layout_id, part_id, rect_id) = seeded_rect().await;
    assert_eq!(
        post_json(
            state.clone(),
            &format!("/design/{layout_id}/object/{rect_id}/delete"),
            ""
        )
        .await,
        StatusCode::OK
    );
    assert!(!object_ids(&state, part_id).contains(&rect_id));

    let body = format!(
        r##"{{"objects":[{{"id":{rect_id},"partId":{part_id},"kind":"rect","x":7,"y":9,"w":40,"h":40,"z":0,"readOnly":false,"binding":null,"content":null,"props":"{{\"fill\":\"#102030\",\"radius\":6}}"}}],"rec":null}}"##
    );
    let (status, resp) = post_json_body(
        state.clone(),
        &format!("/design/{layout_id}/object/restore"),
        &body,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "restore 200\n{resp}");

    // Same id, back in the part.
    assert!(object_ids(&state, part_id).contains(&rect_id));
    // Geometry + props survived: the model re-derives the shape_style.
    let (_, model) = get_body(state.clone(), &format!("/design/{layout_id}/model")).await;
    assert!(
        model.contains("background:#102030;border-radius:6px;"),
        "restored props drive shape_style\n{model}"
    );
}

/// #84 restore rejects an id already in use (reused by an intervening create):
/// 409 and the live row is untouched — never clobbered.
#[tokio::test]
async fn design_object_restore_rejects_id_in_use() {
    let (state, layout_id, part_id, rect_id) = seeded_rect().await;
    let before = object_ids(&state, part_id);
    let body = format!(
        r##"{{"objects":[{{"id":{rect_id},"partId":{part_id},"kind":"rect","x":0,"y":0,"w":10,"h":10,"z":0,"readOnly":false,"binding":null,"content":null,"props":null}}],"rec":null}}"##
    );
    assert_eq!(
        post_json(
            state.clone(),
            &format!("/design/{layout_id}/object/restore"),
            &body
        )
        .await,
        StatusCode::CONFLICT
    );
    assert_eq!(object_ids(&state, part_id), before, "live row untouched");
}

/// #84 restore rejects a part not in the layout: 404, nothing written.
#[tokio::test]
async fn design_object_restore_rejects_unknown_part() {
    let (state, layout_id, part_id, rect_id) = seeded_rect().await;
    post_json(
        state.clone(),
        &format!("/design/{layout_id}/object/{rect_id}/delete"),
        "",
    )
    .await;
    let bogus_part = part_id + 9999;
    let body = format!(
        r##"{{"objects":[{{"id":{rect_id},"partId":{bogus_part},"kind":"rect","x":0,"y":0,"w":10,"h":10,"z":0,"readOnly":false,"binding":null,"content":null,"props":null}}],"rec":null}}"##
    );
    assert_eq!(
        post_json(
            state.clone(),
            &format!("/design/{layout_id}/object/restore"),
            &body
        )
        .await,
        StatusCode::NOT_FOUND
    );
    assert!(
        !object_ids(&state, part_id).contains(&rect_id),
        "nothing written"
    );
}

/// #84 restore is atomic: a valid object followed by one referencing a bad part
/// rolls the whole batch back — the field+label pair never half-restores.
#[tokio::test]
async fn design_object_restore_is_atomic() {
    let (state, layout_id, part_id, rect_id) = seeded_rect().await;
    post_json(
        state.clone(),
        &format!("/design/{layout_id}/object/{rect_id}/delete"),
        "",
    )
    .await;
    let free_id = rect_id + 1000; // unused rowid for the second (doomed) object
    let bogus_part = part_id + 9999;
    let body = format!(
        r##"{{"objects":[{{"id":{rect_id},"partId":{part_id},"kind":"rect","x":0,"y":0,"w":10,"h":10,"z":0,"readOnly":false,"binding":null,"content":null,"props":null}},{{"id":{free_id},"partId":{bogus_part},"kind":"rect","x":0,"y":0,"w":10,"h":10,"z":0,"readOnly":false,"binding":null,"content":null,"props":null}}],"rec":null}}"##
    );
    assert_eq!(
        post_json(
            state.clone(),
            &format!("/design/{layout_id}/object/restore"),
            &body
        )
        .await,
        StatusCode::NOT_FOUND
    );
    let ids = object_ids(&state, part_id);
    assert!(
        !ids.contains(&rect_id),
        "first object rolled back with the batch"
    );
    assert!(!ids.contains(&free_id));
}

/// #15 round-trip: POSTing new geometry persists to `meta_object` (scoped to
/// the layout) and is visible on the next read; bad ids 404 and change nothing;
/// negative coordinates clamp to the canvas origin.
#[tokio::test]
async fn design_object_geometry_persists_clamps_and_is_scoped() {
    let mut sol = Solution::open_in_memory().unwrap();
    sol.create_table(
        "Customers",
        &[NewField {
            name: "Name".into(),
            kind: FieldKind::Text,
        }],
    )
    .unwrap();
    let layout_id = sol.layouts().unwrap()[0].id;
    let part = body_part(&sol, layout_id);
    let obj_id = sol.objects(part.id).unwrap()[0].id;
    let state = state_for(sol);

    let geom = |state: &AppState| {
        let sol = state.sol.lock().unwrap();
        let o = &sol.objects(part.id).unwrap()[0];
        (o.x, o.y, o.w, o.h)
    };

    // A drag commit persists and round-trips.
    let status = post_json(
        state.clone(),
        &format!("/design/{layout_id}/object/{obj_id}/geometry"),
        r#"{"x":33,"y":44,"w":120,"h":30}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(geom(&state), (33, 44, 120, 30));

    // Negative coordinates clamp to the origin (and size to a 1px floor).
    let status = post_json(
        state.clone(),
        &format!("/design/{layout_id}/object/{obj_id}/geometry"),
        r#"{"x":-50,"y":-9,"w":0,"h":-3}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(geom(&state), (0, 0, 1, 1));

    // Unknown object ⇒ 404.
    let status = post_json(
        state.clone(),
        &format!("/design/{layout_id}/object/999999/geometry"),
        r#"{"x":1,"y":1,"w":1,"h":1}"#,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Foreign layout id ⇒ 404 (scoped); geometry unchanged.
    let status = post_json(
        state.clone(),
        &format!("/design/{}/object/{obj_id}/geometry", layout_id + 999),
        r#"{"x":5,"y":5,"w":5,"h":5}"#,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(geom(&state), (0, 0, 1, 1));
}

/// The `/design/:layout/model` JSON is the read contract the canvas hydrates
/// from; pin it to a committed fixture so the Svelte side renders the same
/// model. Also sanity-checks the shape inline.
#[tokio::test]
async fn design_model_endpoint_matches_committed_fixture() {
    let (sol, layout_id) = parity_fixture();
    let (status, body) =
        get_body(state_for(sol), &format!("/design/{layout_id}/model?rec=1")).await;
    assert_eq!(status, StatusCode::OK);
    // Shape sanity (independent of the golden), so a contract change is loud.
    for needle in [
        r#""width":320"#,
        r#""kind":"field""#,
        r#""kind":"text""#,
        r#""kind":"rect""#,
        r#""readOnly":true"#,
        r#""binding":"Customers.Name""#,
        r#""value":"Ada""#,
        r#""content":"Name""#,
        r#""content":"Note""#,
        r#""shape":true"#,
        r#""shapeStyle":"background:#eef;box-shadow:0 0 0 1px #88a;border-radius:4px;""#,
        r#""z":5"#,
    ] {
        assert!(body.contains(needle), "model JSON missing {needle}\n{body}");
    }
    assert_or_regen("canvas.fixture.json", &body);
}

#[tokio::test]
async fn design_model_related_routes_are_derived_from_fk_constraints() {
    let mut sol = Solution::open_in_memory().unwrap();
    let customers = sol
        .create_table(
            "Customers",
            &[
                NewField {
                    name: "Name".into(),
                    kind: FieldKind::Text,
                },
                NewField {
                    name: "RegionId".into(),
                    kind: FieldKind::Number,
                },
            ],
        )
        .unwrap();
    let orders = sol
        .create_table(
            "Orders",
            &[
                NewField {
                    name: "CustomerId".into(),
                    kind: FieldKind::Number,
                },
                NewField {
                    name: "Total".into(),
                    kind: FieldKind::Number,
                },
            ],
        )
        .unwrap();
    let regions = sol
        .create_table(
            "Regions",
            &[NewField {
                name: "Name".into(),
                kind: FieldKind::Text,
            }],
        )
        .unwrap();
    let payments = sol
        .create_table(
            "Payments",
            &[NewField {
                name: "OrderId".into(),
                kind: FieldKind::Number,
            }],
        )
        .unwrap();

    let customer_fields = sol.fields(customers).unwrap();
    let order_fields = sol.fields(orders).unwrap();
    let region_fields = sol.fields(regions).unwrap();
    let payment_fields = sol.fields(payments).unwrap();

    sol.create_relationship(&NewRelationship {
        name: "orders".into(),
        from_table: orders,
        to_table: customers,
        from_field: order_fields[0].id,
        to_field: customer_fields[0].id,
    })
    .unwrap()
    .unwrap();
    sol.create_relationship(&NewRelationship {
        name: "region".into(),
        from_table: customers,
        to_table: regions,
        from_field: customer_fields[1].id,
        to_field: region_fields[0].id,
    })
    .unwrap()
    .unwrap();
    sol.create_relationship(&NewRelationship {
        name: "payments".into(),
        from_table: payments,
        to_table: orders,
        from_field: payment_fields[0].id,
        to_field: order_fields[0].id,
    })
    .unwrap()
    .unwrap();

    let layout_id = sol
        .layouts_for_table(customers)
        .unwrap()
        .into_iter()
        .find(|l| l.view == "form")
        .unwrap()
        .id;
    let (status, body) =
        get_body(state_for(sol), &format!("/design/{layout_id}/model?rec=1")).await;
    assert_eq!(status, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let routes = json["relatedRoutes"].as_array().unwrap();
    assert_eq!(routes.len(), 2, "{routes:#?}");

    let orders_route = routes
        .iter()
        .find(|route| route["name"] == "orders")
        .expect("reverse orders route");
    assert_eq!(orders_route["direction"], "reverse");
    assert_eq!(orders_route["cardinality"], "toMany");
    assert_eq!(orders_route["tableId"], orders);
    assert_eq!(orders_route["tableName"], "Orders");
    assert_eq!(orders_route["fromTable"], orders);
    assert_eq!(orders_route["toTable"], customers);

    let region_route = routes
        .iter()
        .find(|route| route["name"] == "region")
        .expect("forward region route");
    assert_eq!(region_route["direction"], "forward");
    assert_eq!(region_route["cardinality"], "toOne");
    assert_eq!(region_route["tableId"], regions);
    assert_eq!(region_route["tableName"], "Regions");

    assert!(
        routes.iter().all(|route| route["name"] != "payments"),
        "routes from unrelated tables must not be offered"
    );
}

/// Browse renders the parity fixture's canvas; this is the canonical band DOM
/// the Svelte `LayoutPreview` must reproduce (the macro is the spec).
#[tokio::test]
async fn browse_canvas_matches_parity_golden() {
    let (sol, layout_id) = parity_fixture();
    let (status, html) = get_body(state_for(sol), &format!("/browse/{layout_id}?view=form")).await;
    assert_eq!(status, StatusCode::OK);
    // The form holds exactly one `.fm-canvas`; slice it out up to `</form>`.
    let start = html
        .find(r#"<div class="fm-canvas""#)
        .expect("canvas present");
    let end = start + html[start..].find("</form>").expect("form closes");
    let canvas = normalize_html(&html[start..end]);
    assert!(canvas.starts_with(r#"<div class="fm-canvas""#) && canvas.ends_with("</div>"));
    assert_or_regen("canvas.parity.html", &canvas);
}

/// Value formatting (#77/#78) must reach ALL Browse views — including Table,
/// which renders a field-derived grid that used to bypass the formatter. The
/// editable input DISPLAYS the formatted value but carries the RAW value in
/// data-raw so committing never writes the formatted string back (#80 guard).
#[tokio::test]
async fn browse_applies_value_format_in_form_list_and_table() {
    let mut sol = Solution::open_in_memory().unwrap();
    let tid = sol
        .create_table(
            "Invoices",
            &[
                NewField {
                    name: "Total".into(),
                    kind: FieldKind::Number,
                },
                NewField {
                    name: "Due".into(),
                    kind: FieldKind::Date,
                },
            ],
        )
        .unwrap();
    let table = sol.table_by_name("Invoices").unwrap().unwrap();
    let fields = sol.fields(tid).unwrap();
    sol.insert_record(
        &table,
        &[
            (&fields[0], "1234.5".into()),
            (&fields[1], "12/25/2003".into()),
        ],
    )
    .unwrap();
    // Set formats on every layout's field objects.
    sol.app
        .execute(
            "UPDATE meta_object SET props=?1 WHERE binding='Invoices.Total'",
            [r#"{"format":{"mode":"decimal","fixedDecimals":true,"decimalDigits":2,"thousandsSeparator":","}}"#],
        )
        .unwrap();
    sol.app
        .execute(
            "UPDATE meta_object SET props=?1 WHERE binding='Invoices.Due'",
            [r#"{"format":{"mode":"predefined","predefined":"yyyy-mm-dd"}}"#],
        )
        .unwrap();
    let layouts = sol.layouts().unwrap();
    let by_view = |v: &str| {
        layouts
            .iter()
            .find(|l| canonical_view(&l.view) == v)
            .map(|l| l.id)
    };
    let (form, list, table_l) = (
        by_view("form").unwrap(),
        by_view("list").unwrap(),
        by_view("table").unwrap(),
    );
    let state = state_for(sol);

    for (label, lid) in [("form", form), ("list", list), ("table", table_l)] {
        let (status, html) = get_body(state.clone(), &format!("/browse/{lid}")).await;
        assert_eq!(status, StatusCode::OK, "{label} renders");
        assert!(
            html.contains("1,234.50"),
            "{label} shows the formatted value"
        );
        assert!(
            html.contains("2003-12-25"),
            "{label} shows the formatted date"
        );
        // The raw value must ride data-raw (so the editable input commits raw),
        // not be the visible/committed default.
        assert!(
            html.contains(r#"data-raw="1234.5""#),
            "{label} keeps the raw value in data-raw for safe commit"
        );
        assert!(
            html.contains(r#"data-raw="12/25/2003""#),
            "{label} keeps the raw date in data-raw for safe commit"
        );
    }
}
