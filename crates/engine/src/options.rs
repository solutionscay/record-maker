//! The field options bag (`meta_field.options`) as TYPED policy (#128 theme D).
//!
//! The engine stores options as an opaque JSON string; this module owns the two
//! rule families the engine itself enforces:
//!
//! * **Validation** — required / unique / range / member-of-value-list, parsed
//!   into [`FieldOptions`]/[`ValidationRules`] and enforced inside
//!   [`Solution::insert_record`]/[`Solution::update_record`] via
//!   [`Solution::validate_record_values`]. Failures are the typed
//!   [`ValidationError`]; HTTP consumers map it to a status + its `Display`
//!   message (the message text is a UI contract — keep it stable).
//! * **Reference constraints** — the `reference` key that mirrors a declared
//!   relationship row. The relationship CRUD ops in `model.rs` keep the key in
//!   step transactionally; [`with_reference`]/[`without_reference`] are the one
//!   definition of that key's shape.
//!
//! Parsing is deliberately LENIENT (bad JSON / wrong-typed keys read as "no
//! rule"), matching how the options bag has always been read; the stored bytes
//! are never rewritten by validation.

use std::collections::{HashMap, HashSet};
use std::fmt;

use serde_json::{Map, Value};

use crate::model::{FieldKind, FieldMeta, RelationshipMeta, TableMeta};
use crate::Solution;

/// Lenient parse of a raw options string: empty or invalid JSON reads as `{}`.
pub fn options_value(options: &str) -> Value {
    if options.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        serde_json::from_str::<Value>(options).unwrap_or_else(|_| Value::Object(Map::new()))
    }
}

impl FieldMeta {
    /// This field's options bag as a JSON value (lenient — see [`options_value`]).
    pub fn options_value(&self) -> Value {
        options_value(&self.options)
    }

    /// Whether this is the table's system primary key (#156) — auto-minted,
    /// value-immutable, undeletable. See [`FieldOptions::system`].
    pub fn is_system(&self) -> bool {
        FieldOptions::parse(&self.options).system
    }
}

/// Typed view of a field's options bag — the keys the engine enforces.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldOptions {
    /// The `validation` sub-object, when present and well-formed.
    pub validation: Option<ValidationRules>,
    /// The `autoEnter` sub-object (#159/#160): a value the engine populates when
    /// a record is created and the field is left empty. Extensible per source;
    /// only the constant source exists today.
    pub auto_enter: Option<AutoEnter>,
    /// The system primary key (#156): auto-minted UUID, value-immutable,
    /// undeletable, fixed-kind. Set once at table creation; users never toggle it.
    pub system: bool,
}

/// An auto-enter value source (#159). Stored under `options.autoEnter` as
/// `{"kind":"<source>", …}`; unknown/malformed sources read as `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoEnter {
    /// A static default: fill this constant when the field is left empty (#160).
    Constant { value: String },
}

/// The `validation` rules of one field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationRules {
    /// Primary-key marker: implies required + unique.
    pub primary: bool,
    pub required: bool,
    pub unique: bool,
    /// Inclusive min/max bounds (numeric for Number fields, lexicographic for
    /// the ISO-8601 temporal kinds).
    pub range: Option<RangeRule>,
    /// Id of a value list every submitted line must be a member of.
    pub member_of_value_list: Option<i64>,
}

/// An inclusive range rule; empty/blank bounds read as unbounded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RangeRule {
    pub min: Option<String>,
    pub max: Option<String>,
}

impl FieldOptions {
    /// Parse a raw options string leniently: a missing/non-object `validation`
    /// means no rules; wrong-typed keys read as their defaults.
    pub fn parse(options: &str) -> Self {
        let value = options_value(options);
        let validation = value
            .get("validation")
            .filter(|v| v.is_object())
            .map(|v| ValidationRules {
                primary: bool_rule(v, "primary"),
                required: bool_rule(v, "required"),
                unique: bool_rule(v, "unique"),
                range: v.get("range").filter(|r| r.is_object()).map(|r| RangeRule {
                    min: string_rule(r, "min"),
                    max: string_rule(r, "max"),
                }),
                member_of_value_list: v.get("memberOfValueList").and_then(Value::as_i64),
            });
        let auto_enter = value
            .get("autoEnter")
            .filter(|v| v.is_object())
            .and_then(|v| match v.get("kind").and_then(Value::as_str) {
                Some("constant") => Some(AutoEnter::Constant {
                    value: v.get("value").and_then(Value::as_str).unwrap_or("").to_string(),
                }),
                _ => None,
            });
        FieldOptions {
            validation,
            auto_enter,
            system: bool_rule(&value, "system"),
        }
    }
}

fn bool_rule(rule: &Value, key: &str) -> bool {
    rule.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn string_rule(rule: &Value, key: &str) -> Option<String> {
    rule.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// A record write rejected by a field's validation rules. `Display` is the
/// user-facing message (rendered by the UI) — its wording is a contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    Required { field: String },
    NotUnique { field: String },
    NotANumber { field: String },
    InvalidMinimum { field: String },
    InvalidMaximum { field: String },
    BelowMinimum { field: String, min: String },
    AboveMaximum { field: String, max: String },
    UnknownValueList { field: String },
    NotInValueList { field: String },
    /// A storage-layer failure surfaced while checking a rule (kept as a
    /// validation outcome so consumers report it the same way they always have).
    Storage { message: String },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Required { field } => write!(f, "Field \"{field}\" is required."),
            Self::NotUnique { field } => write!(f, "Field \"{field}\" must be unique."),
            Self::NotANumber { field } => write!(f, "Field \"{field}\" must be a number."),
            Self::InvalidMinimum { field } => {
                write!(f, "Field \"{field}\" has an invalid minimum.")
            }
            Self::InvalidMaximum { field } => {
                write!(f, "Field \"{field}\" has an invalid maximum.")
            }
            Self::BelowMinimum { field, min } => {
                write!(f, "Field \"{field}\" must be at least {min}.")
            }
            Self::AboveMaximum { field, max } => {
                write!(f, "Field \"{field}\" must be at most {max}.")
            }
            Self::UnknownValueList { field } => {
                write!(f, "Field \"{field}\" references an unknown value list.")
            }
            Self::NotInValueList { field } => {
                write!(f, "Field \"{field}\" must be a member of its value list.")
            }
            Self::Storage { message } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ValidationError {}

fn storage(e: anyhow::Error) -> ValidationError {
    ValidationError::Storage {
        message: e.to_string(),
    }
}

/// How strictly a record write is validated (#173 — the draft lifecycle).
///
/// A record is minted as a DRAFT and only the record-EXIT commit enforces the
/// full rule set. Both modes still enforce type/format/range/member-of-value-list
/// on any *present* value; `Draft` only defers presence ([`ValidationError::Required`])
/// and uniqueness ([`ValidationError::NotUnique`]) so a blank New record can be
/// minted and the user can tab between fields before completing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationMode {
    /// The record-EXIT commit gate: enforce every rule.
    Full,
    /// A draft write: defer required + unique to the eventual commit.
    Draft,
}

impl Solution {
    /// Validate submitted `(field, value)` pairs against every field's rules.
    /// Ran by [`Solution::insert_record`] / [`Solution::update_record`] (with
    /// `existing_id` excluding the row being updated from uniqueness checks), so
    /// no engine consumer can bypass validation. In [`ValidationMode::Full`] a
    /// field not submitted at all still fails its required rule; in
    /// [`ValidationMode::Draft`] required + unique are deferred (see
    /// [`ValidationMode`]).
    pub fn validate_record_values(
        &self,
        table: &TableMeta,
        values: &[(&FieldMeta, String)],
        existing_id: Option<i64>,
        mode: ValidationMode,
    ) -> Result<(), ValidationError> {
        let full = matches!(mode, ValidationMode::Full);
        let fields = self.fields(table.id).map_err(storage)?;
        let submitted: HashMap<i64, &str> =
            values.iter().map(|(f, v)| (f.id, v.as_str())).collect();
        for field in &fields {
            let Some(rules) = FieldOptions::parse(&field.options).validation else {
                continue;
            };
            let value = submitted.get(&field.id).copied();
            let trimmed = value.unwrap_or("").trim();
            if full && (rules.primary || rules.required) && (value.is_none() || trimmed.is_empty()) {
                return Err(ValidationError::Required {
                    field: field.name.clone(),
                });
            }

            if value.is_none() || trimmed.is_empty() {
                continue;
            }

            if full
                && (rules.primary || rules.unique)
                && self
                    .field_value_exists(table, field, trimmed, existing_id)
                    .map_err(storage)?
            {
                return Err(ValidationError::NotUnique {
                    field: field.name.clone(),
                });
            }

            if let Some(range) = &rules.range {
                validate_range(field, trimmed, range)?;
            }

            if let Some(value_list_id) = rules.member_of_value_list {
                self.validate_member_of_value_list(field, value_list_id, trimmed)?;
            }
        }
        Ok(())
    }

    fn validate_member_of_value_list(
        &self,
        field: &FieldMeta,
        value_list_id: i64,
        value: &str,
    ) -> Result<(), ValidationError> {
        let items = self
            .resolve_value_list(value_list_id)
            .map_err(storage)?
            .ok_or_else(|| ValidationError::UnknownValueList {
                field: field.name.clone(),
            })?;
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
                return Err(ValidationError::NotInValueList {
                    field: field.name.clone(),
                });
            }
        }
        Ok(())
    }
}

fn validate_range(field: &FieldMeta, value: &str, range: &RangeRule) -> Result<(), ValidationError> {
    let (min, max) = (range.min.as_deref(), range.max.as_deref());
    if min.is_none() && max.is_none() {
        return Ok(());
    }
    match field.kind {
        FieldKind::Number => {
            let parsed = value
                .parse::<f64>()
                .map_err(|_| ValidationError::NotANumber {
                    field: field.name.clone(),
                })?;
            if let Some(min) = min {
                let min = min
                    .parse::<f64>()
                    .map_err(|_| ValidationError::InvalidMinimum {
                        field: field.name.clone(),
                    })?;
                if parsed < min {
                    return Err(ValidationError::BelowMinimum {
                        field: field.name.clone(),
                        min: min.to_string(),
                    });
                }
            }
            if let Some(max) = max {
                let max = max
                    .parse::<f64>()
                    .map_err(|_| ValidationError::InvalidMaximum {
                        field: field.name.clone(),
                    })?;
                if parsed > max {
                    return Err(ValidationError::AboveMaximum {
                        field: field.name.clone(),
                        max: max.to_string(),
                    });
                }
            }
        }
        FieldKind::Date | FieldKind::Time | FieldKind::Timestamp => {
            if let Some(min) = min {
                if value < min {
                    return Err(ValidationError::BelowMinimum {
                        field: field.name.clone(),
                        min: min.to_string(),
                    });
                }
            }
            if let Some(max) = max {
                if value > max {
                    return Err(ValidationError::AboveMaximum {
                        field: field.name.clone(),
                        max: max.to_string(),
                    });
                }
            }
        }
        FieldKind::Text | FieldKind::Bool => {}
    }
    Ok(())
}

// ── reference constraints ───────────────────────────────────────────────────

/// A field's declared reference constraint (the `reference` options key):
/// "this field holds keys of `to_field` on `to_table`, via the relationship
/// named `name`". The input side of [`Solution::set_field_reference`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldReference {
    pub name: String,
    pub to_table: i64,
    pub to_field: i64,
}

/// Why a [`Solution::set_field_reference`] call could not be applied. `Display`
/// is the user-facing message consumers have always shown for each case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldReferenceError {
    SourceFieldMissing,
    TargetFieldMissing,
    RelationshipFieldsMissing,
    /// Another relationship already uses this name. Names are the route tokens
    /// `Solution::resolve_path` matches on (case-insensitively), so they must be
    /// globally unique or a portal/related-list route can't be addressed.
    DuplicateName,
}

impl fmt::Display for FieldReferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SourceFieldMissing => "source field not found",
            Self::TargetFieldMissing => "target field not found",
            Self::RelationshipFieldsMissing => "relationship fields not found",
            Self::DuplicateName => "a relationship with this name already exists",
        })
    }
}

impl std::error::Error for FieldReferenceError {}

/// Return `options` with its `reference` key set from `rel` — the one
/// definition of that key's shape (also used by read-side projections that
/// overlay the live relationship onto a field's options).
pub fn with_reference(mut options: Value, rel: &RelationshipMeta) -> Value {
    let Some(obj) = options.as_object_mut() else {
        return options;
    };
    obj.insert(
        "reference".to_string(),
        serde_json::json!({
            "name": rel.name,
            "toTable": rel.to_table,
            "toField": rel.to_field,
        }),
    );
    options
}

/// Return `options` with its `reference` key removed.
pub fn without_reference(mut options: Value) -> Value {
    if let Some(obj) = options.as_object_mut() {
        obj.remove("reference");
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NewField, NewValueList};

    #[test]
    fn field_options_parse_is_lenient_and_typed() {
        assert_eq!(FieldOptions::parse(""), FieldOptions::default());
        assert_eq!(FieldOptions::parse("not json"), FieldOptions::default());
        assert_eq!(
            FieldOptions::parse(r#"{"validation":"yes"}"#),
            FieldOptions::default()
        );

        let parsed = FieldOptions::parse(
            r#"{"validation":{"primary":true,"range":{"min":" 1 ","max":""},"memberOfValueList":7}}"#,
        );
        let rules = parsed.validation.unwrap();
        assert!(rules.primary && !rules.required && !rules.unique);
        assert_eq!(
            rules.range,
            Some(RangeRule {
                min: Some("1".into()),
                max: None,
            })
        );
        assert_eq!(rules.member_of_value_list, Some(7));
        assert_eq!(parsed.auto_enter, None);

        // autoEnter parses the constant source; unknown/malformed sources read as None.
        assert_eq!(
            FieldOptions::parse(r#"{"autoEnter":{"kind":"constant","value":"Open"}}"#).auto_enter,
            Some(AutoEnter::Constant { value: "Open".into() })
        );
        assert_eq!(
            FieldOptions::parse(r#"{"autoEnter":{"kind":"serial"}}"#).auto_enter,
            None
        );
        // A constant with no value reads as an empty string, not a parse failure.
        assert_eq!(
            FieldOptions::parse(r#"{"autoEnter":{"kind":"constant"}}"#).auto_enter,
            Some(AutoEnter::Constant { value: String::new() })
        );
    }

    #[test]
    fn record_writes_enforce_validation_rules_with_stable_messages() {
        let mut s = Solution::open_in_memory().unwrap();
        let tid = s
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
        let table = s.table_by_id(tid).unwrap().unwrap();
        let fields = s.fields(tid).unwrap();
        let list = s
            .create_value_list(&NewValueList {
                name: "Statuses".into(),
                source: "custom".into(),
                config: r#"{"values":["Open","Closed"]}"#.into(),
            })
            .unwrap();
        // Required + unique (not primary): a primary field would auto-fill a
        // UUID on insert (see the primary_key_autofills_uuid test), so it can no
        // longer surface the "required"/"unique" messages this test checks.
        s.update_field_options(
            tid,
            fields[0].id,
            r#"{"validation":{"required":true,"unique":true}}"#,
        )
        .unwrap();
        s.update_field_options(
            tid,
            fields[1].id,
            r#"{"validation":{"range":{"min":"1","max":"10"}}}"#,
        )
        .unwrap();
        s.update_field_options(
            tid,
            fields[2].id,
            &format!(r#"{{"validation":{{"memberOfValueList":{}}}}}"#, list.id),
        )
        .unwrap();
        let fields = s.fields(tid).unwrap();
        let row = |n: &str, t: &str, st: &str| {
            vec![
                (&fields[0], n.to_string()),
                (&fields[1], t.to_string()),
                (&fields[2], st.to_string()),
            ]
        };
        let msg = |r: anyhow::Result<i64>| {
            r.unwrap_err()
                .downcast::<ValidationError>()
                .unwrap()
                .to_string()
        };

        assert_eq!(
            msg(s.insert_record(&table, &row("", "5", "Open"))),
            "Field \"Number\" is required."
        );
        assert_eq!(
            msg(s.insert_record(&table, &row("INV-1", "15", "Open"))),
            "Field \"Total\" must be at most 10."
        );
        assert_eq!(
            msg(s.insert_record(&table, &row("INV-1", "5", "Draft"))),
            "Field \"Status\" must be a member of its value list."
        );

        let id = s
            .insert_record(&table, &row("INV-1", "5", "Open\nClosed"))
            .unwrap();
        assert_eq!(
            msg(s.insert_record(&table, &row("INV-1", "6", "Closed"))),
            "Field \"Number\" must be unique."
        );

        // Updates exclude the row itself from uniqueness but keep every rule.
        s.update_record(&table, id, &row("INV-1", "7", "Closed"))
            .unwrap();
        let err = s
            .update_record(&table, id, &row("INV-1", "0", "Closed"))
            .unwrap_err()
            .downcast::<ValidationError>()
            .unwrap();
        assert_eq!(err.to_string(), "Field \"Total\" must be at least 1.");
    }
}
