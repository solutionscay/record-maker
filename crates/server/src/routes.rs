//! HTTP handlers grouped by surface: Browse pages, record actions, the
//! schema JSON API, and the Layout-Mode design API. The router itself stays
//! in the crate root (`app`).

pub(crate) mod browse;
pub(crate) mod design;
pub(crate) mod records;
pub(crate) mod schema;
