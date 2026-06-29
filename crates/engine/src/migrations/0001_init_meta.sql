-- 0001_init_meta — the initial metadata model (app.db).
-- See ADR-0001 (real tables), ADR-0002 (two-db split), ADR-0003 (no table
-- occurrences; named-path relationships), ADR-0004 (versioned format).
-- APPEND-ONLY once shipped: evolve via new migrations, never by editing this.

-- A user table. `phys_name` is the real table name created in data.db.
CREATE TABLE meta_table (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL UNIQUE,          -- logical name, e.g. "Customers"
    phys_name  TEXT NOT NULL UNIQUE,          -- physical table name in data.db
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- A field on a user table. `phys_name` is the real column name in data.db.
CREATE TABLE meta_field (
    id        INTEGER PRIMARY KEY,
    table_id  INTEGER NOT NULL REFERENCES meta_table(id) ON DELETE CASCADE,
    name      TEXT NOT NULL,                  -- logical field name
    phys_name TEXT NOT NULL,                  -- physical column name
    kind      TEXT NOT NULL,                  -- logical type: text|number|date|bool|...
    calc      TEXT,                           -- calc expression, if a calc field
    options   TEXT,                           -- JSON: validation, auto-enter, etc.
    position  INTEGER NOT NULL DEFAULT 0,
    UNIQUE (table_id, name),
    UNIQUE (table_id, phys_name)
);

-- A NAMED association between two tables (ADR-0003): no table occurrences.
-- Two relationships to the same table coexist by `name` (e.g. bill_to / ship_to).
CREATE TABLE meta_relationship (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL,                 -- path segment, e.g. "bill_to"
    from_table INTEGER NOT NULL REFERENCES meta_table(id) ON DELETE CASCADE,
    to_table   INTEGER NOT NULL REFERENCES meta_table(id) ON DELETE CASCADE,
    from_field INTEGER REFERENCES meta_field(id) ON DELETE SET NULL,
    to_field   INTEGER REFERENCES meta_field(id) ON DELETE SET NULL,
    UNIQUE (from_table, name)
);

-- A layout binds to a primary TABLE (not an occurrence) — ADR-0003.
CREATE TABLE meta_layout (
    id       INTEGER PRIMARY KEY,
    name     TEXT NOT NULL,
    table_id INTEGER NOT NULL REFERENCES meta_table(id) ON DELETE CASCADE,
    view     TEXT NOT NULL DEFAULT 'form'     -- form|list|table
);

-- A layout part (band): header|body|footer|subsummary|grandsummary.
CREATE TABLE meta_part (
    id        INTEGER PRIMARY KEY,
    layout_id INTEGER NOT NULL REFERENCES meta_layout(id) ON DELETE CASCADE,
    kind      TEXT NOT NULL,
    height    INTEGER NOT NULL DEFAULT 100,
    position  INTEGER NOT NULL DEFAULT 0
);

-- An object on a part. Absolute geometry (pixel-perfect) + a dot-path binding
-- like "Invoice.bill_to.name" (ADR-0003).
CREATE TABLE meta_object (
    id      INTEGER PRIMARY KEY,
    part_id INTEGER NOT NULL REFERENCES meta_part(id) ON DELETE CASCADE,
    kind    TEXT NOT NULL,                    -- field|text|button|portal|...
    x       INTEGER NOT NULL DEFAULT 0,
    y       INTEGER NOT NULL DEFAULT 0,
    w       INTEGER NOT NULL DEFAULT 100,
    h       INTEGER NOT NULL DEFAULT 24,
    binding TEXT,                             -- dot-path expression
    props   TEXT                             -- JSON: label, style ref, etc.
);

CREATE INDEX idx_meta_field_table        ON meta_field(table_id);
CREATE INDEX idx_meta_relationship_from  ON meta_relationship(from_table);
CREATE INDEX idx_meta_layout_table       ON meta_layout(table_id);
CREATE INDEX idx_meta_part_layout        ON meta_part(layout_id);
CREATE INDEX idx_meta_object_part        ON meta_object(part_id);
