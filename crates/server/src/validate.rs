//! HTTP-side record-form helpers. The validation rules themselves live in the
//! engine (`record_maker_engine::options`, enforced inside `insert_record` /
//! `update_record` — #128 theme D); this module only bridges the Browse form
//! contract to it: collecting `f<field_id>` inputs and translating a rejected
//! write into the 400 + message the handlers have always returned.

use std::collections::HashMap;

use record_maker_engine::FieldMeta;

/// Pull `f<field_id>` form values into engine `(field, value)` pairs.
pub(crate) fn collect_values<'a>(
    fields: &'a [FieldMeta],
    form: &HashMap<String, String>,
) -> Vec<(&'a FieldMeta, String)> {
    fields
        .iter()
        .filter_map(|f| form.get(&format!("f{}", f.id)).map(|v| (f, v.clone())))
        .collect()
}
