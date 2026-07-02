//! Display-only value formatting (#77 number/Boolean, #78 date/time).
//!
//! A field object may carry a `format` bag inside its `props` JSON (the same bag
//! the style helpers read). This module turns a **raw stored value** plus that
//! `format` spec plus the bound field's [`FieldKind`] into the string Browse and
//! the Layout canvas display. It is purely presentational — it never touches the
//! stored data, so an unset/empty/unparseable spec always returns the raw value
//! unchanged (the "leave data as entered" default).
//!
//! The negative-number color (#77) can't live in the static per-object props CSS
//! because it depends on the *value*, so [`format_value`] surfaces it separately
//! as [`Formatted::color`]; the caller appends it to the object's inline text
//! style so a negative value paints in its configured color.
//!
//! Japanese / Kanji numeral + date display is intentionally out of scope (both
//! issues defer it).

use record_maker_engine::FieldKind;
use serde_json::Value;

/// A formatted display value plus an optional value-dependent text color (the
/// negative-number color, #77). `color` is `None` for everything else.
pub struct Formatted {
    pub text: String,
    pub color: Option<String>,
}

impl Formatted {
    fn plain(text: impl Into<String>) -> Self {
        Formatted {
            text: text.into(),
            color: None,
        }
    }
}

/// Format `raw` for display given the object's `format` spec (the `format`
/// sub-value of its props bag, or `None` when absent) and the bound field kind.
/// Returns the raw value untouched whenever no formatting applies.
pub fn format_value(raw: &str, format: Option<&Value>, kind: FieldKind) -> Formatted {
    match kind {
        FieldKind::Number | FieldKind::Bool => format_number(raw, format),
        FieldKind::Date => format_date(raw, format),
        FieldKind::Time => format_time(raw, format),
        FieldKind::Timestamp => format_timestamp(raw, format),
        FieldKind::Text => Formatted::plain(raw),
    }
}

// ---- small typed readers over the (untrusted) format bag ----

fn get_str<'a>(v: &'a Value, key: &str, default: &'a str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or(default)
}

fn get_str_opt<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn get_bool(v: &Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn get_i64(v: &Value, key: &str, default: i64) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(default)
}

// ---------------------------------------------------------------------------
// Number / Boolean (#77)
// ---------------------------------------------------------------------------

fn format_number(raw: &str, format: Option<&Value>) -> Formatted {
    let Some(f) = format else {
        return Formatted::plain(raw);
    };
    match get_str(f, "mode", "general") {
        "asEntered" => Formatted::plain(raw),
        "boolean" => {
            let non_zero = raw.trim().parse::<f64>().map(|n| n != 0.0).unwrap_or(false);
            let text = if non_zero {
                get_str(f, "booleanNonZero", "")
            } else {
                get_str(f, "booleanZero", "")
            };
            Formatted::plain(text)
        }
        "decimal" => format_numeric(raw, f, true),
        // "general" (and any unknown mode) → numeric with separators/negatives
        // but no fixed decimals or currency.
        _ => format_numeric(raw, f, false),
    }
}

/// Core numeric renderer shared by `general` and `decimal` modes. `decimal_mode`
/// enables the fixed-decimal, currency and hide-if-zero controls; both modes get
/// the decimal/thousands separators and negative formatting.
fn format_numeric(raw: &str, f: &Value, decimal_mode: bool) -> Formatted {
    let Ok(num) = raw.trim().parse::<f64>() else {
        return Formatted::plain(raw);
    };

    if decimal_mode && get_bool(f, "hideZero", false) && num == 0.0 {
        return Formatted::plain("");
    }

    let decimals: Option<usize> = if decimal_mode && get_bool(f, "fixedDecimals", false) {
        Some(get_i64(f, "decimalDigits", 2).clamp(0, 15) as usize)
    } else {
        None
    };
    let dec_sep = get_str(f, "decimalSeparator", ".");
    let thou_sep = get_str(f, "thousandsSeparator", "");
    let neg_style = get_str(f, "negativeStyle", "minus");
    let currency = if decimal_mode {
        get_str(f, "currency", "none")
    } else {
        "none"
    };
    let symbol = get_str(f, "currencySymbol", "");

    let negative = num < 0.0;
    let abs = num.abs();
    let base = match decimals {
        Some(d) => format!("{abs:.d$}"),
        // f64 Display never uses scientific notation and round-trips shortest.
        None => format!("{abs}"),
    };

    let (int_part, frac_part) = match base.split_once('.') {
        Some((i, fr)) => (i.to_string(), Some(fr.to_string())),
        None => (base, None),
    };
    let grouped = if thou_sep.is_empty() {
        int_part
    } else {
        group_thousands(&int_part, thou_sep)
    };
    let mut digits = grouped;
    if let Some(fr) = frac_part {
        digits.push_str(dec_sep);
        digits.push_str(&fr);
    }

    // Assemble symbol + sign. "inside" tucks the symbol against the digits
    // (inside the sign/parens: `-$1,234` / `($1,234)`); "leading" keeps it
    // outermost (`$-1,234` / `$(1,234)`).
    let mut out = digits;
    if currency == "inside" {
        out = format!("{symbol}{out}");
    }
    if negative {
        out = if neg_style == "parens" {
            format!("({out})")
        } else {
            format!("-{out}")
        };
    }
    if currency == "leading" {
        out = format!("{symbol}{out}");
    }

    let color = if negative {
        get_str_opt(f, "negativeColor").map(str::to_string)
    } else {
        None
    };
    Formatted { text: out, color }
}

/// Insert `sep` every three digits from the right of an unsigned digit string.
fn group_thousands(int_digits: &str, sep: &str) -> String {
    let len = int_digits.len();
    let mut out = String::with_capacity(len + len / 3 * sep.len());
    for (i, ch) in int_digits.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push_str(sep);
        }
        out.push(ch);
    }
    out
}

// ---------------------------------------------------------------------------
// Date (#78)
// ---------------------------------------------------------------------------

struct DateParts {
    y: i32,
    mo: u32,
    d: u32,
}

/// Parse the date portion of an ISO value (`YYYY-MM-DD`, or the date half of a
/// `YYYY-MM-DDThh:mm:ss` timestamp). For Browse-entered values, also accept the
/// common slash form `MM/DD/YYYY` (or `DD/MM/YYYY` when the first segment is > 12).
fn parse_date(raw: &str) -> Option<DateParts> {
    let date = raw.trim().split(['T', ' ']).next()?;
    let (y, mo, d) = if date.contains('-') {
        let mut it = date.split('-');
        (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?)
    } else if date.contains('/') {
        let parts = date
            .split('/')
            .map(str::parse::<i32>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        if parts.len() != 3 {
            return None;
        }
        let y = normalize_year(parts[2]);
        if parts[0] > 12 {
            (y, parts[1] as u32, parts[0] as u32)
        } else {
            (y, parts[0] as u32, parts[1] as u32)
        }
    } else {
        return None;
    };
    // Reject out-of-range components so a malformed-but-numeric value (e.g.
    // "2013-13-01") degrades to the raw string instead of panicking downstream:
    // weekday()/month_name() index fixed 12-element tables by month.
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    Some(DateParts { y, mo, d })
}

fn normalize_year(y: i32) -> i32 {
    if (0..=69).contains(&y) {
        2000 + y
    } else if (70..=99).contains(&y) {
        1900 + y
    } else {
        y
    }
}

const MONTHS_LONG: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAYS_LONG: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const WEEKDAYS_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// Day of week, 0 = Sunday .. 6 = Saturday (Sakamoto's algorithm).
fn weekday(mut y: i32, m: u32, d: u32) -> usize {
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    if m < 3 {
        y -= 1;
    }
    let m = m as i32;
    let d = d as i32;
    let w = (y + y / 4 - y / 100 + y / 400 + T[(m - 1) as usize] + d) % 7;
    (((w % 7) + 7) % 7) as usize
}

fn format_date(raw: &str, format: Option<&Value>) -> Formatted {
    let Some(f) = format else {
        return Formatted::plain(raw);
    };
    let mode = get_str(f, "mode", "asEntered");
    if mode == "asEntered" {
        return Formatted::plain(raw);
    }
    let Some(dp) = parse_date(raw) else {
        return Formatted::plain(raw);
    };
    match mode {
        "predefined" => Formatted::plain(render_predefined_date(&dp, f)),
        "custom" => Formatted::plain(render_custom_date(&dp, f)),
        _ => Formatted::plain(raw),
    }
}

fn render_predefined_date(dp: &DateParts, f: &Value) -> String {
    let name = get_str(f, "predefined", "mm/dd/yyyy");
    let default_sep = if name.contains('-') { "-" } else { "/" };
    let sep = get_str(f, "dateSeparator", default_sep);
    let mm = format!("{:02}", dp.mo);
    let dd = format!("{:02}", dp.d);
    let yy = format!("{:02}", (dp.y % 100 + 100) % 100);
    let yyyy = dp.y.to_string();
    match name {
        "mm/dd/yy" => format!("{mm}{sep}{dd}{sep}{yy}"),
        "dd/mm/yy" => format!("{dd}{sep}{mm}{sep}{yy}"),
        "dd/mm/yyyy" => format!("{dd}{sep}{mm}{sep}{yyyy}"),
        "yyyy-mm-dd" => format!("{yyyy}{sep}{mm}{sep}{dd}"),
        // "mm/dd/yyyy" and any unknown name.
        _ => format!("{mm}{sep}{dd}{sep}{yyyy}"),
    }
}

/// Render the `components` array in order. Each component prepends its optional
/// `leading` text (the per-component separator), then its own value.
fn render_custom_date(dp: &DateParts, f: &Value) -> String {
    let Some(comps) = f.get("components").and_then(Value::as_array) else {
        return String::new();
    };
    let mut out = String::new();
    for c in comps {
        out.push_str(get_str(c, "leading", ""));
        match get_str(c, "type", "") {
            "dayOfWeek" => {
                let idx = weekday(dp.y, dp.mo, dp.d);
                let name = if get_str(c, "style", "long") == "short" {
                    WEEKDAYS_SHORT[idx]
                } else {
                    WEEKDAYS_LONG[idx]
                };
                out.push_str(name);
            }
            "month" => match get_str(c, "style", "number") {
                "long" => out.push_str(month_name(dp.mo, &MONTHS_LONG)),
                "short" => out.push_str(month_name(dp.mo, &MONTHS_SHORT)),
                _ => {
                    if get_bool(c, "leadingZero", true) {
                        out.push_str(&format!("{:02}", dp.mo));
                    } else {
                        out.push_str(&dp.mo.to_string());
                    }
                }
            },
            "day" => {
                if get_bool(c, "leadingZero", false) {
                    out.push_str(&format!("{:02}", dp.d));
                } else {
                    out.push_str(&dp.d.to_string());
                }
            }
            "year" => {
                if get_str(c, "style", "full") == "short" {
                    out.push_str(&format!("{:02}", (dp.y % 100 + 100) % 100));
                } else {
                    out.push_str(&dp.y.to_string());
                }
            }
            _ => {}
        }
    }
    out
}

fn month_name(mo: u32, names: &[&'static str; 12]) -> &'static str {
    let idx = (mo.clamp(1, 12) - 1) as usize;
    names[idx]
}

// ---------------------------------------------------------------------------
// Time (#78)
// ---------------------------------------------------------------------------

struct TimeParts {
    h: u32,
    mi: u32,
    s: u32,
}

/// Parse the time portion of an ISO value (`hh:mm:ss`, or the time half of a
/// `YYYY-MM-DDThh:mm:ss` timestamp). Seconds default to 0 when absent.
fn parse_time(raw: &str) -> Option<TimeParts> {
    let t = raw.trim().rsplit(['T', ' ']).next()?;
    let mut it = t.split(':');
    let h = it.next()?.parse().ok()?;
    let mi = it.next()?.parse().ok()?;
    let s = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    Some(TimeParts { h, mi, s })
}

fn format_time(raw: &str, format: Option<&Value>) -> Formatted {
    let Some(f) = format else {
        return Formatted::plain(raw);
    };
    let mode = get_str(f, "mode", "asEntered");
    if mode == "asEntered" {
        return Formatted::plain(raw);
    }
    let Some(tp) = parse_time(raw) else {
        return Formatted::plain(raw);
    };
    let show_seconds = match mode {
        "predefined" => get_str(f, "predefined", "hh:mm:ss").contains("ss"),
        "custom" => get_bool(f, "showSeconds", true),
        _ => return Formatted::plain(raw),
    };
    Formatted::plain(render_time(&tp, f, show_seconds))
}

fn render_time(tp: &TimeParts, f: &Value, show_seconds: bool) -> String {
    let h24 = get_bool(f, "hours24", true);
    let sep = get_str(f, "timeSeparator", ":");
    let h_zero = get_bool(f, "hoursLeadingZero", true);
    let ms_zero = get_bool(f, "minutesSecondsLeadingZero", true);

    let (disp_h, am) = if h24 {
        (tp.h, None)
    } else {
        let am = tp.h < 12;
        let mut hh = tp.h % 12;
        if hh == 0 {
            hh = 12;
        }
        (hh, Some(am))
    };

    let hstr = if h_zero {
        format!("{disp_h:02}")
    } else {
        disp_h.to_string()
    };
    let mstr = if ms_zero {
        format!("{:02}", tp.mi)
    } else {
        tp.mi.to_string()
    };
    let mut out = format!("{hstr}{sep}{mstr}");
    if show_seconds {
        let sstr = if ms_zero {
            format!("{:02}", tp.s)
        } else {
            tp.s.to_string()
        };
        out.push_str(sep);
        out.push_str(&sstr);
    }

    if let Some(am) = am {
        let placement = get_str(f, "amPmPlacement", "after");
        if placement != "none" {
            let label = if am {
                get_str(f, "amLabel", "AM")
            } else {
                get_str(f, "pmLabel", "PM")
            };
            out = if placement == "before" {
                format!("{label} {out}")
            } else {
                format!("{out} {label}")
            };
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Timestamp (#78) — the date panel + the time panel together.
// ---------------------------------------------------------------------------

fn format_timestamp(raw: &str, format: Option<&Value>) -> Formatted {
    let Some(f) = format else {
        return Formatted::plain(raw);
    };
    let date_spec = f.get("date");
    let time_spec = f.get("time");
    if date_spec.is_none() && time_spec.is_none() {
        return Formatted::plain(raw);
    }
    // Split once so each sub-panel formats (or echoes) only its own half; an
    // `asEntered` sub-panel then returns just that half, not the whole value.
    let date_part = raw.trim().split(['T', ' ']).next().unwrap_or("");
    let time_part = raw
        .trim()
        .find(['T', ' '])
        .map(|i| &raw.trim()[i + 1..])
        .unwrap_or("");
    let sep = get_str(f, "separator", " ");
    let date_str = format_date(date_part, date_spec).text;
    let time_str = format_time(time_part, time_spec).text;
    Formatted::plain(format!("{date_str}{sep}{time_str}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fmt(raw: &str, spec: Value, kind: FieldKind) -> String {
        format_value(raw, Some(&spec), kind).text
    }

    // ---- number ----

    #[test]
    fn number_no_spec_is_raw() {
        assert_eq!(format_value("1234.5", None, FieldKind::Number).text, "1234.5");
    }

    #[test]
    fn number_as_entered() {
        assert_eq!(
            fmt("1234.50", json!({"mode": "asEntered"}), FieldKind::Number),
            "1234.50"
        );
    }

    #[test]
    fn number_general_with_thousands() {
        assert_eq!(
            fmt(
                "1234567.5",
                json!({"mode": "general", "thousandsSeparator": ","}),
                FieldKind::Number
            ),
            "1,234,567.5"
        );
    }

    #[test]
    fn decimal_fixed_digits() {
        assert_eq!(
            fmt(
                "1234.5",
                json!({"mode": "decimal", "fixedDecimals": true, "decimalDigits": 2}),
                FieldKind::Number
            ),
            "1234.50"
        );
    }

    #[test]
    fn decimal_currency_leading_and_thousands() {
        assert_eq!(
            fmt(
                "1234.5",
                json!({
                    "mode": "decimal", "fixedDecimals": true, "decimalDigits": 2,
                    "thousandsSeparator": ",", "currency": "leading", "currencySymbol": "$"
                }),
                FieldKind::Number
            ),
            "$1,234.50"
        );
    }

    #[test]
    fn decimal_currency_inside_negative_minus() {
        // inside → symbol tucked against digits, inside the sign.
        assert_eq!(
            fmt(
                "-1234",
                json!({
                    "mode": "decimal", "fixedDecimals": true, "decimalDigits": 0,
                    "thousandsSeparator": ",", "currency": "inside", "currencySymbol": "$"
                }),
                FieldKind::Number
            ),
            "-$1,234"
        );
    }

    #[test]
    fn decimal_currency_leading_negative_parens() {
        // leading → symbol outermost, wrapping the parens.
        assert_eq!(
            fmt(
                "-1234",
                json!({
                    "mode": "decimal", "fixedDecimals": true, "decimalDigits": 0,
                    "thousandsSeparator": ",", "currency": "leading", "currencySymbol": "$",
                    "negativeStyle": "parens"
                }),
                FieldKind::Number
            ),
            "$(1,234)"
        );
    }

    #[test]
    fn negative_parens_no_currency() {
        assert_eq!(
            fmt(
                "-1234",
                json!({"mode": "general", "negativeStyle": "parens"}),
                FieldKind::Number
            ),
            "(1234)"
        );
    }

    #[test]
    fn negative_minus_default() {
        assert_eq!(
            fmt("-1234", json!({"mode": "general"}), FieldKind::Number),
            "-1234"
        );
    }

    #[test]
    fn negative_color_surfaces() {
        let out = format_value(
            "-5",
            Some(&json!({"mode": "general", "negativeColor": "#ff0000"})),
            FieldKind::Number,
        );
        assert_eq!(out.text, "-5");
        assert_eq!(out.color.as_deref(), Some("#ff0000"));
        // Positive value carries no color even when configured.
        let pos = format_value(
            "5",
            Some(&json!({"mode": "general", "negativeColor": "#ff0000"})),
            FieldKind::Number,
        );
        assert_eq!(pos.color, None);
    }

    #[test]
    fn custom_decimal_separator() {
        assert_eq!(
            fmt(
                "1234.5",
                json!({
                    "mode": "decimal", "fixedDecimals": true, "decimalDigits": 2,
                    "decimalSeparator": ",", "thousandsSeparator": "."
                }),
                FieldKind::Number
            ),
            "1.234,50"
        );
    }

    #[test]
    fn decimal_hide_zero() {
        assert_eq!(
            fmt(
                "0",
                json!({"mode": "decimal", "hideZero": true, "fixedDecimals": true, "decimalDigits": 2}),
                FieldKind::Number
            ),
            ""
        );
        // Non-zero unaffected.
        assert_eq!(
            fmt(
                "3",
                json!({"mode": "decimal", "hideZero": true, "fixedDecimals": true, "decimalDigits": 2}),
                FieldKind::Number
            ),
            "3.00"
        );
    }

    #[test]
    fn boolean_mode() {
        let spec = json!({"mode": "boolean", "booleanNonZero": "Yes", "booleanZero": "No"});
        assert_eq!(fmt("1", spec.clone(), FieldKind::Bool), "Yes");
        assert_eq!(fmt("0", spec.clone(), FieldKind::Bool), "No");
        assert_eq!(fmt("", spec.clone(), FieldKind::Bool), "No");
        assert_eq!(fmt("5", spec, FieldKind::Number), "Yes");
    }

    #[test]
    fn number_unparseable_is_raw() {
        assert_eq!(
            fmt("n/a", json!({"mode": "decimal", "fixedDecimals": true}), FieldKind::Number),
            "n/a"
        );
    }

    // ---- date ----

    #[test]
    fn date_as_entered() {
        assert_eq!(
            fmt("2003-12-25", json!({"mode": "asEntered"}), FieldKind::Date),
            "2003-12-25"
        );
    }

    #[test]
    fn date_predefined_mmddyy() {
        assert_eq!(
            fmt(
                "2003-12-25",
                json!({"mode": "predefined", "predefined": "mm/dd/yy"}),
                FieldKind::Date
            ),
            "12/25/03"
        );
    }

    #[test]
    fn date_predefined_iso_and_separator() {
        assert_eq!(
            fmt(
                "2003-12-25",
                json!({"mode": "predefined", "predefined": "yyyy-mm-dd", "dateSeparator": "."}),
                FieldKind::Date
            ),
            "2003.12.25"
        );
    }

    #[test]
    fn date_predefined_accepts_slash_entered_dates() {
        let spec = json!({"mode": "predefined", "predefined": "yyyy-mm-dd"});
        assert_eq!(fmt("12/25/2003", spec.clone(), FieldKind::Date), "2003-12-25");
        assert_eq!(fmt("25/12/2003", spec.clone(), FieldKind::Date), "2003-12-25");
        assert_eq!(fmt("1/5/03", spec, FieldKind::Date), "2003-01-05");
    }

    #[test]
    fn date_predefined_ddmmyyyy() {
        assert_eq!(
            fmt(
                "2003-12-25",
                json!({"mode": "predefined", "predefined": "dd/mm/yyyy"}),
                FieldKind::Date
            ),
            "25/12/2003"
        );
    }

    #[test]
    fn date_custom_components() {
        // Thursday, December 25, 2003
        let spec = json!({
            "mode": "custom",
            "components": [
                {"type": "dayOfWeek", "style": "long"},
                {"type": "month", "style": "long", "leading": ", "},
                {"type": "day", "leading": " "},
                {"type": "year", "style": "full", "leading": ", "}
            ]
        });
        assert_eq!(
            fmt("2003-12-25", spec, FieldKind::Date),
            "Thursday, December 25, 2003"
        );
    }

    #[test]
    fn date_custom_numeric_components_leading_zero() {
        let spec = json!({
            "mode": "custom",
            "components": [
                {"type": "month", "style": "number", "leadingZero": true},
                {"type": "day", "leadingZero": true, "leading": "/"},
                {"type": "year", "style": "short", "leading": "/"}
            ]
        });
        assert_eq!(fmt("2003-01-05", spec, FieldKind::Date), "01/05/03");
    }

    #[test]
    fn weekday_known_dates() {
        // 2003-12-25 was a Thursday; 2000-01-01 a Saturday.
        assert_eq!(WEEKDAYS_LONG[weekday(2003, 12, 25)], "Thursday");
        assert_eq!(WEEKDAYS_LONG[weekday(2000, 1, 1)], "Saturday");
    }

    #[test]
    fn malformed_date_degrades_to_raw_without_panic() {
        // Out-of-range month/day must not reach weekday()/month_name()'s fixed
        // 12-element tables (would panic and poison the shared render lock).
        let spec = json!({
            "mode": "custom",
            "components": [{ "type": "dayOfWeek", "style": "long" }]
        });
        assert_eq!(fmt("2013-13-01", spec.clone(), FieldKind::Date), "2013-13-01");
        assert_eq!(fmt("2003-00-05", spec.clone(), FieldKind::Date), "2003-00-05");
        assert_eq!(fmt("2003-01-40", spec, FieldKind::Date), "2003-01-40");
    }

    // ---- time ----

    #[test]
    fn time_as_entered() {
        assert_eq!(
            fmt("13:05:09", json!({"mode": "asEntered"}), FieldKind::Time),
            "13:05:09"
        );
    }

    #[test]
    fn time_predefined_24h() {
        assert_eq!(
            fmt(
                "13:05:09",
                json!({"mode": "predefined", "predefined": "hh:mm:ss"}),
                FieldKind::Time
            ),
            "13:05:09"
        );
    }

    #[test]
    fn time_predefined_hhmm_drops_seconds() {
        assert_eq!(
            fmt(
                "13:05:09",
                json!({"mode": "predefined", "predefined": "hh:mm"}),
                FieldKind::Time
            ),
            "13:05"
        );
    }

    #[test]
    fn time_12h_pm_after() {
        assert_eq!(
            fmt(
                "13:05:09",
                json!({"mode": "custom", "hours24": false, "showSeconds": true}),
                FieldKind::Time
            ),
            "01:05:09 PM"
        );
    }

    #[test]
    fn time_12h_am_labels_before_no_leading_zero() {
        assert_eq!(
            fmt(
                "09:05:00",
                json!({
                    "mode": "custom", "hours24": false, "showSeconds": false,
                    "hoursLeadingZero": false, "amPmPlacement": "before", "amLabel": "a.m."
                }),
                FieldKind::Time
            ),
            "a.m. 9:05"
        );
    }

    #[test]
    fn time_midnight_12h() {
        assert_eq!(
            fmt(
                "00:00:00",
                json!({"mode": "custom", "hours24": false, "showSeconds": false}),
                FieldKind::Time
            ),
            "12:00 AM"
        );
    }

    #[test]
    fn time_custom_separator_and_no_ampm() {
        assert_eq!(
            fmt(
                "13:05:09",
                json!({
                    "mode": "custom", "hours24": false, "showSeconds": true,
                    "timeSeparator": ".", "amPmPlacement": "none"
                }),
                FieldKind::Time
            ),
            "01.05.09"
        );
    }

    #[test]
    fn time_minutes_seconds_no_leading_zero() {
        assert_eq!(
            fmt(
                "13:05:09",
                json!({
                    "mode": "custom", "hours24": true, "showSeconds": true,
                    "minutesSecondsLeadingZero": false
                }),
                FieldKind::Time
            ),
            "13:5:9"
        );
    }

    // ---- timestamp ----

    #[test]
    fn timestamp_no_spec_is_raw() {
        assert_eq!(
            format_value("2003-12-25T13:05:09", None, FieldKind::Timestamp).text,
            "2003-12-25T13:05:09"
        );
    }

    #[test]
    fn timestamp_date_and_time_panels() {
        let spec = json!({
            "date": {"mode": "predefined", "predefined": "mm/dd/yy"},
            "time": {"mode": "predefined", "predefined": "hh:mm"},
            "separator": " "
        });
        assert_eq!(
            fmt("2003-12-25T13:05:09", spec, FieldKind::Timestamp),
            "12/25/03 13:05"
        );
    }

    #[test]
    fn timestamp_as_entered_halves() {
        // date panel formats, time panel left as entered → echoes its half only.
        let spec = json!({
            "date": {"mode": "predefined", "predefined": "yyyy-mm-dd"},
            "time": {"mode": "asEntered"}
        });
        assert_eq!(
            fmt("2003-12-25T13:05:09", spec, FieldKind::Timestamp),
            "2003-12-25 13:05:09"
        );
    }

    #[test]
    fn text_kind_never_formats() {
        assert_eq!(
            format_value("-1234", Some(&json!({"mode": "decimal"})), FieldKind::Text).text,
            "-1234"
        );
    }
}
