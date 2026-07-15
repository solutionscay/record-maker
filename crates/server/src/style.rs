//! Server-derived inline-CSS strings for layout objects and parts (#49):
//! the single derivation both the askama band macro and the Svelte canvas
//! interpolate (the #44 parity contract), so there is no second copy to drift.

use record_maker_engine::ObjectKind;

#[derive(Clone, Copy, Default)]
struct StrokeSides {
    top: bool,
    right: bool,
    bottom: bool,
    left: bool,
    middle: bool,
}

impl StrokeSides {
    fn all_outer(self) -> bool {
        self.top && self.right && self.bottom && self.left
    }
}

/// `None` is the legacy contract: all four outer edges. An explicit array,
/// including `[]`, is an authored #191 placement. Unknown entries are ignored.
fn stroke_sides(v: &serde_json::Value) -> Option<StrokeSides> {
    let values = v.get("strokeSides")?.as_array()?;
    let mut sides = StrokeSides::default();
    for value in values.iter().filter_map(serde_json::Value::as_str) {
        match value {
            "top" => sides.top = true,
            "right" => sides.right = true,
            "bottom" => sides.bottom = true,
            "left" => sides.left = true,
            "middle" => sides.middle = true,
            _ => {}
        }
    }
    Some(sides)
}

fn stroke_value<'a>(v: &'a serde_json::Value, fallback: &'a str) -> &'a str {
    v.get("stroke")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(fallback)
}

fn stroke_width(v: &serde_json::Value, fallback: i64) -> i64 {
    v.get("strokeWidth")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(fallback)
        .max(0)
}

/// Append the #191 box-border projection. Returns true when the placement is
/// explicit, letting callers avoid also emitting the legacy uniform ring.
fn append_border_placement(
    s: &mut String,
    v: &serde_json::Value,
    fallback_color: &str,
    allow_middle: bool,
) -> bool {
    let Some(sides) = stroke_sides(v) else {
        return false;
    };
    let color = stroke_value(v, fallback_color);
    let width = stroke_width(v, 1);

    if sides.all_outer() {
        // Keep the established all-edge renderer byte-for-byte. This makes an
        // explicit All visually match the legacy absent-metadata fallback.
        if v.get("stroke")
            .and_then(serde_json::Value::as_str)
            .is_some()
        {
            s.push_str(&format!("box-shadow:0 0 0 {width}px {color};"));
        }
    } else {
        // Suppress each kind's default uniform frame and expose edge widths to
        // the interaction-transparent CSS overlay. Geometry never changes.
        s.push_str("border:0;box-shadow:none;");
        s.push_str(&format!(
            "--fm-stroke-color:{color};--fm-stroke-top:{}px;--fm-stroke-right:{}px;--fm-stroke-bottom:{}px;--fm-stroke-left:{}px;",
            if sides.top { width } else { 0 },
            if sides.right { width } else { 0 },
            if sides.bottom { width } else { 0 },
            if sides.left { width } else { 0 },
        ));
    }

    if allow_middle && sides.middle {
        if sides.all_outer() {
            s.push_str(&format!("--fm-stroke-color:{color};"));
        }
        s.push_str(&format!("--fm-stroke-middle:{width}px;"));
    }
    true
}

pub(crate) fn parse_props(props: Option<&str>) -> Option<serde_json::Value> {
    let Some(props) = props else {
        return None;
    };
    serde_json::from_str::<serde_json::Value>(props).ok()
}

/// Derive a shape object's inline CSS from its `props` JSON. #49 owns the full
/// appearance contract; this reads the keys a rect/line/ellipse needs — `fill`,
/// `stroke`, `strokeWidth`, `strokeSides`, `radius`. The string is computed once here and carried
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
    let explicit_placement = if matches!(kind, ObjectKind::Rect) {
        append_border_placement(&mut s, &v, "#d3d8de", false)
    } else {
        false
    };
    if !explicit_placement {
        if let Some(stroke) = v.get("stroke").and_then(serde_json::Value::as_str) {
            let width = stroke_width(&v, 1);
            // Render the stroke OUTSIDE the box (box-shadow ring) so a thicker stroke
            // grows the object visually without eating into its stored geometry; the
            // ring also follows `border-radius`, so ellipses stay round (#44 issue 2).
            s.push_str(&format!("box-shadow:0 0 0 {width}px {stroke};"));
        }
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
    let explicit_placement = match kind {
        ObjectKind::Field => append_border_placement(&mut s, &v, "#b9c2cc", false),
        ObjectKind::Portal => append_border_placement(&mut s, &v, "#b9c2cc", true),
        _ => false,
    };
    if !explicit_placement {
        if let Some(stroke) = v.get("stroke").and_then(serde_json::Value::as_str) {
            let width = stroke_width(&v, 1);
            // Stroke grows outward (box-shadow ring) rather than inward, so geometry is
            // preserved and a thicker border makes the object visually bigger (issue 2).
            s.push_str(&format!("box-shadow:0 0 0 {width}px {stroke};"));
        }
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
/// The kind gate reads the engine's per-kind capability table so "takes text
/// formatting" has exactly one definition.
pub(crate) fn text_style(kind: ObjectKind, props: Option<&str>) -> String {
    if !kind.capabilities().text_format {
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

#[cfg(test)]
mod border_placement_tests {
    use super::*;

    #[test]
    fn legacy_missing_placement_keeps_uniform_outer_ring() {
        assert_eq!(
            object_style(
                ObjectKind::Field,
                Some(r##"{"stroke":"#123456","strokeWidth":3}"##),
            ),
            "box-shadow:0 0 0 3px #123456;"
        );
    }

    #[test]
    fn rectangle_projects_independent_edges_without_geometry_changes() {
        assert_eq!(
            shape_style(
                ObjectKind::Rect,
                Some(
                    r##"{"fill":"#ffffff","stroke":"#123456","strokeWidth":2,"strokeSides":["top","right"]}"##,
                ),
            ),
            "background:#ffffff;border:0;box-shadow:none;--fm-stroke-color:#123456;--fm-stroke-top:2px;--fm-stroke-right:2px;--fm-stroke-bottom:0px;--fm-stroke-left:0px;"
        );
        assert_eq!(
            shape_style(
                ObjectKind::Rect,
                Some(r##"{"stroke":"#123456","strokeWidth":2,"strokeSides":[]}"##,),
            ),
            "border:0;box-shadow:none;--fm-stroke-color:#123456;--fm-stroke-top:0px;--fm-stroke-right:0px;--fm-stroke-bottom:0px;--fm-stroke-left:0px;"
        );
    }

    #[test]
    fn portal_middle_is_independent_from_every_outer_edge() {
        assert_eq!(
            object_style(
                ObjectKind::Portal,
                Some(
                    r##"{"stroke":"#123456","strokeWidth":2,"strokeSides":["middle"]}"##,
                ),
            ),
            "border:0;box-shadow:none;--fm-stroke-color:#123456;--fm-stroke-top:0px;--fm-stroke-right:0px;--fm-stroke-bottom:0px;--fm-stroke-left:0px;--fm-stroke-middle:2px;"
        );
        assert_eq!(
            object_style(
                ObjectKind::Portal,
                Some(
                    r##"{"stroke":"#123456","strokeWidth":2,"strokeSides":["top","right","bottom","left","middle"]}"##,
                ),
            ),
            "box-shadow:0 0 0 2px #123456;--fm-stroke-color:#123456;--fm-stroke-middle:2px;"
        );
    }

    #[test]
    fn ellipse_and_line_ignore_box_edge_metadata() {
        assert_eq!(
            shape_style(
                ObjectKind::Ellipse,
                Some(r##"{"stroke":"#123456","strokeWidth":2,"strokeSides":["top"]}"##,),
            ),
            "box-shadow:0 0 0 2px #123456;"
        );
        assert_eq!(
            shape_style(
                ObjectKind::Line,
                Some(r##"{"stroke":"#123456","strokeWidth":2,"strokeSides":[]}"##,),
            ),
            "background:#123456;height:2px;"
        );
    }
}
