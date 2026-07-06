-- 0008_schema_notes — user-facing notes on schema metadata.
-- Tables and fields can carry descriptive notes edited from the schema builder.

ALTER TABLE meta_table ADD COLUMN notes TEXT NOT NULL DEFAULT '';
ALTER TABLE meta_field ADD COLUMN notes TEXT NOT NULL DEFAULT '';
