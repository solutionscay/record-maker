//! Server-derived inline-CSS strings for layout objects and parts (#49):
//! the single derivation both the askama band macro and the Svelte canvas
//! interpolate (the #44 parity contract), so there is no second copy to drift.

use record_maker_engine::ObjectKind;

pub(crate) fn parse_props(props: Option<&str>) -> Option<serde_json::Value> {
    let Some(props) = props else {
        return None;
    };
    serde_json::from_str::<serde_json::Value>(props).ok()
}

/// Derive a shape object's inline CSS from its `props` JSON. #49 owns the full
/// appearance contract; this reads the keys a rect/line/ellipse needs — `fill`,
/// `stroke`, `strokeWidth`, `radius`. The string is computed once here and carried
/// in [`ObjectView::shape_style`], so the askama band macro and the Svelte `Band`
/// both just interpolate it — there is no second derivation to keep byte-equal.
/// Empty for absent/invalid props (an unstyled shape falls back to its CSS class).
pub(crate) fn shape_style(kind: ObjectKind, props: Option<&str>) -> String {
    let Some(v) = parse_props(props) else {
        return String::new();
    };
    let mut s = String::new();
    // A line is a 1-D shape: `stroke` is its COLOUR and `strokeWidth` its THICKNESS
    // — rendered as a centred bar by the `.fm-line` rule, not the outer ring rects
    // use. (The ring would be clipped by `.fm-obj { overflow:hidden }` and could not
    // grow a line's weight, which is why the Border control appeared to do nothing.)
    if matches!(kind, ObjectKind::Line) {
        let stroke = v
            .get("stroke")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("#888");
        let width = v
            .get("strokeWidth")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(2)
            .max(1);
        s.push_str(&format!("background:{stroke};height:{width}px;"));
        if v.get("angle").is_some() || v.get("length").is_some() {
            let angle = v
                .get("angle")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let length = v
                .get("length")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(1.0)
                .max(1.0);
            s.push_str(&format!(
                "width:{length}px;left:50%;right:auto;transform:translate(-50%,-50%) rotate({angle}deg);transform-origin:center center;"
            ));
        }
        return s;
    }
    if let Some(fill) = v.get("fill").and_then(serde_json::Value::as_str) {
        s.push_str(&format!("background:{fill};"));
    }
    if let Some(stroke) = v.get("stroke").and_then(serde_json::Value::as_str) {
        let width = v
            .get("strokeWidth")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(1);
        // Render the stroke OUTSIDE the box (box-shadow ring) so a thicker stroke
        // grows the object visually without eating into its stored geometry; the
        // ring also follows `border-radius`, so ellipses stay round (#44 issue 2).
        s.push_str(&format!("box-shadow:0 0 0 {width}px {stroke};"));
    }
    if let Some(radius) = v.get("radius").and_then(serde_json::Value::as_i64) {
        s.push_str(&format!("border-radius:{radius}px;"));
    }
    s
}

/// Box-level style for non-shape layout objects. Field objects use this for fill
/// and border; text objects accept the same props if present, but the first UI
/// pass exposes text formatting for text boxes rather than fill/line controls.
pub(crate) fn object_style(kind: ObjectKind, props: Option<&str>) -> String {
    if kind.is_shape() {
        return String::new();
    }
    let Some(v) = parse_props(props) else {
        return String::new();
    };
    let mut s = String::new();
    if let Some(fill) = v.get("fill").and_then(serde_json::Value::as_str) {
        s.push_str(&format!("background:{fill};"));
    }
    if let Some(stroke) = v.get("stroke").and_then(serde_json::Value::as_str) {
        let width = v
            .get("strokeWidth")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(1)
            .max(0);
        // Stroke grows outward (box-shadow ring) rather than inward, so geometry is
        // preserved and a thicker border makes the object visually bigger (issue 2).
        s.push_str(&format!("box-shadow:0 0 0 {width}px {stroke};"));
    }
    if let Some(radius) = v.get("radius").and_then(serde_json::Value::as_i64) {
        s.push_str(&format!("border-radius:{}px;", radius.max(0)));
    }
    s
}

/// Derive a part/band's inline CSS from its `props` JSON (#49/Issue 7), mirroring
/// [`object_style`]. The band's only appearance today is a background `fill`; the
/// derived string is computed once here and interpolated identically by the askama
/// band macro and the Svelte `Band` (the #44 parity contract). Empty for
/// absent/invalid props (an unstyled band falls back to its `.fm-part` class).
pub(crate) fn part_style(props: Option<&str>) -> String {
    let Some(v) = parse_props(props) else {
        return String::new();
    };
    let mut s = String::new();
    if let Some(fill) = v.get("fill").and_then(serde_json::Value::as_str) {
        s.push_str(&format!("background:{fill};"));
    }
    s
}

/// Text-level style for field and text objects. Alignment includes flex
/// justification because field display values are vertically-centered flex spans.
pub(crate) fn text_style(kind: ObjectKind, props: Option<&str>) -> String {
    if !matches!(kind, ObjectKind::Field | ObjectKind::Text) {
        return String::new();
    }
    let Some(v) = parse_props(props) else {
        return String::new();
    };
    let mut s = String::new();
    if let Some(color) = v.get("textColor").and_then(serde_json::Value::as_str) {
        s.push_str(&format!("color:{color};"));
    }
    if let Some(size) = v.get("fontSize").and_then(serde_json::Value::as_i64) {
        s.push_str(&format!("font-size:{}px;", size.clamp(6, 96)));
    }
    if v.get("bold")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        s.push_str("font-weight:700;");
    }
    if v.get("italic")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        s.push_str("font-style:italic;");
    }
    if v.get("underline")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        s.push_str("text-decoration:underline;");
    }
    match v.get("align").and_then(serde_json::Value::as_str) {
        Some("center") => s.push_str("text-align:center;justify-content:center;"),
        Some("right") => s.push_str("text-align:right;justify-content:flex-end;"),
        Some("left") => s.push_str("text-align:left;justify-content:flex-start;"),
        _ => {}
    }
    s
}
