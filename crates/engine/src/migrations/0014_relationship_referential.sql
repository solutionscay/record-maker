-- Portal CRUD anchor metadata on a named relationship (#110).
-- Two referential permission flags gate whether a portal anchored on this
-- relationship may create / delete related records. They are a property of the
-- relationship itself (one permission, no per-portal flag). Existing rows
-- default to the safe state: no create, no delete.
--
-- Cardinality is NOT stored: it is derived from which side holds the FK
-- (from_field lives on from_table), so forward traversal is to-one and reverse
-- is to-many. See RelationshipMeta::forward_cardinality / reverse_cardinality.
ALTER TABLE meta_relationship
    ADD COLUMN allow_create INTEGER NOT NULL DEFAULT 0;
ALTER TABLE meta_relationship
    ADD COLUMN allow_delete INTEGER NOT NULL DEFAULT 0;
