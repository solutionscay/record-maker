-- 0009_value_lists -- reusable solution-level value lists (#101).
--
-- A value list is a named metadata object whose concrete items are resolved at
-- read time. `source` selects the config shape:
--   custom: { "values": ["Small", "Medium", "-", "Large"] }
--   field:  { "fromField": 1, "secondField": null,
--             "showSecondOnly": false, "sort": "first" }
--
-- APPEND-ONLY once shipped (ADR-0004): evolve via new migrations, never by
-- editing this file.

CREATE TABLE meta_value_list (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL UNIQUE,
    source     TEXT NOT NULL,
    config     TEXT NOT NULL,
    position   INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK (source IN ('custom', 'field'))
);

CREATE INDEX idx_meta_value_list_position ON meta_value_list(position, id);
