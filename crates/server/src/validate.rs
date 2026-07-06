//! Record validation: required/unique/range/member-of-value-list rules read
//! from a field's `options` bag, applied on record create/commit. Relocated
//! verbatim from the handler layer; moving it into the engine (typed
//! `FieldOptions`, enforced in `Solution` save/create) is #128 theme D.

use std::collections::{HashMap, HashSet};

use record_maker_engine::{FieldKind, FieldMeta, Solution, TableMeta};
use serde_json::{Map, Value};

pub(crate) fn field_options_value(f: &FieldMeta) -> Value {
    if f.options.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        serde_json::from_str::<Value>(&f.options).unwrap_or_else(|_| Value::Object(Map::new()))
    }
}

/// Pull `f<field_id>` form values into engine `(field, value)` pairs.
pub(crate) fn collect_values<'a>(
    fields: &'a [record_maker_engine::FieldMeta],
    form: &HashMap<String, String>,
) -> Vec<(&'a record_maker_engine::FieldMeta, String)> {
    fields
        .iter()
        .filter_map(|f| form.get(&format!("f{}", f.id)).map(|v| (f, v.clone())))
        .collect()
}

pub(crate) fn validate_record_values(
    sol: &Solution,
    table: &TableMeta,
    fields: &[FieldMeta],
    values: &[(&FieldMeta, String)],
    existing_id: Option<i64>,
) -> Result<(), String> {
    let submitted: HashMap<i64, &str> = values.iter().map(|(f, v)| (f.id, v.as_str())).collect();
    for field in fields {
        let options = field_options_value(field);
        let validation = options.get("validation").unwrap_or(&Value::Null);
        if !validation.is_object() {
            continue;
        }
        let value = submitted.get(&field.id).copied();
        let trimmed = value.unwrap_or("").trim();
        let primary = validation
            .get("primary")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let required = validation
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if (primary || required) && (value.is_none() || trimmed.is_empty()) {
            return Err(format!("Field \"{}\" is required.", field.name));
        }

        if value.is_none() || trimmed.is_empty() {
            continue;
        }

        let unique = validation
            .get("unique")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if (primary || unique)
            && sol
                .field_value_exists(table, field, trimmed, existing_id)
                .map_err(|e| e.to_string())?
        {
            return Err(format!("Field \"{}\" must be unique.", field.name));
        }

        if let Some(range) = validation.get("range").filter(|v| v.is_object()) {
            validate_range(field, trimmed, range)?;
        }

        if let Some(value_list_id) = validation.get("memberOfValueList").and_then(Value::as_i64) {
            validate_member_of_value_list(sol, field, value_list_id, trimmed)?;
        }
    }
    Ok(())
}

fn string_rule<'a>(rule: &'a Value, key: &str) -> Option<&'a str> {
    rule.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn validate_range(field: &FieldMeta, value: &str, range: &Value) -> Result<(), String> {
    let min = string_rule(range, "min");
    let max = string_rule(range, "max");
    if min.is_none() && max.is_none() {
        return Ok(());
    }
    match field.kind {
        FieldKind::Number => {
            let parsed = value
                .parse::<f64>()
                .map_err(|_| format!("Field \"{}\" must be a number.", field.name))?;
            if let Some(min) = min {
                let min = min
                    .parse::<f64>()
                    .map_err(|_| format!("Field \"{}\" has an invalid minimum.", field.name))?;
                if parsed < min {
                    return Err(format!("Field \"{}\" must be at least {min}.", field.name));
                }
            }
            if let Some(max) = max {
                let max = max
                    .parse::<f64>()
                    .map_err(|_| format!("Field \"{}\" has an invalid maximum.", field.name))?;
                if parsed > max {
                    return Err(format!("Field \"{}\" must be at most {max}.", field.name));
                }
            }
        }
        FieldKind::Date | FieldKind::Time | FieldKind::Timestamp => {
            if let Some(min) = min {
                if value < min {
                    return Err(format!("Field \"{}\" must be at least {min}.", field.name));
                }
            }
            if let Some(max) = max {
                if value > max {
                    return Err(format!("Field \"{}\" must be at most {max}.", field.name));
                }
            }
        }
        FieldKind::Text | FieldKind::Bool => {}
    }
    Ok(())
}

fn validate_member_of_value_list(
    sol: &Solution,
    field: &FieldMeta,
    value_list_id: i64,
    value: &str,
) -> Result<(), String> {
    let items = sol
        .resolve_value_list(value_list_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Field \"{}\" references an unknown value list.", field.name))?;
    let allowed: HashSet<String> = items
        .into_iter()
        .filter(|item| !item.divider)
        .map(|item| item.value)
        .collect();
    for part in value
        .split('\n')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        if !allowed.contains(part) {
            return Err(format!(
                "Field \"{}\" must be a member of its value list.",
                field.name
            ));
        }
    }
    Ok(())
}
