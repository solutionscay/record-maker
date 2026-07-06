//! Demo seed data: a "Customers" table with enough field variety to exercise
//! Browse, value formatting, and the schema builder on first run.

use std::collections::HashSet;

use record_maker_engine::{FieldKind, NewField, Solution};

/// Seed a demo "Customers" table on first run so there's something to browse.
pub fn seed(sol: &mut Solution) -> anyhow::Result<()> {
    let customer_fields = demo_customer_fields();
    if sol.tables()?.is_empty() {
        sol.create_table("Customers", &customer_fields)?;
    } else if let Some(table) = sol.table_by_name("Customers")? {
        let existing: HashSet<String> = sol.fields(table.id)?.into_iter().map(|f| f.name).collect();
        for f in customer_fields {
            if !existing.contains(&f.name) {
                sol.add_field(table.id, &f)?;
            }
        }
    }
    Ok(())
}

fn demo_customer_fields() -> Vec<NewField> {
    vec![
        NewField {
            name: "Name".into(),
            kind: FieldKind::Text,
        },
        NewField {
            name: "Email".into(),
            kind: FieldKind::Text,
        },
        NewField {
            name: "Age".into(),
            kind: FieldKind::Number,
        },
        NewField {
            name: "DOB".into(),
            kind: FieldKind::Date,
        },
        NewField {
            name: "Phone".into(),
            kind: FieldKind::Text,
        },
        NewField {
            name: "Street".into(),
            kind: FieldKind::Text,
        },
        NewField {
            name: "City".into(),
            kind: FieldKind::Text,
        },
        NewField {
            name: "State".into(),
            kind: FieldKind::Text,
        },
        NewField {
            name: "ZIP".into(),
            kind: FieldKind::Text,
        },
        NewField {
            name: "Balance".into(),
            kind: FieldKind::Number,
        },
        NewField {
            name: "CreditLimit".into(),
            kind: FieldKind::Number,
        },
        NewField {
            name: "LoyaltyPoints".into(),
            kind: FieldKind::Number,
        },
        NewField {
            name: "DiscountPct".into(),
            kind: FieldKind::Number,
        },
    ]
}
