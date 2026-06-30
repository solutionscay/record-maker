//! record-maker engine — the metadata-driven core.
//!
//! A *solution* is made of two SQLite databases (ADR-0002):
//! - `app.db`  — metadata: fixed, **versioned** schema (ADR-0004)
//! - `data.db` — the user's tables: dynamic schema (ADR-0001)
//!
//! NOTE: `anyhow` is used for error handling during the MVP; it will be
//! replaced with a typed error enum before the engine becomes a public API.

pub mod data;
pub mod db;
pub mod layout;
pub mod model;
pub mod schema;

pub use data::Record;
pub use db::Solution;
pub use layout::{LayoutMeta, NewObject, ObjectKind, ObjectMeta, PartKind, PartMeta};
pub use model::{FieldKind, FieldMeta, NewField, TableMeta};
