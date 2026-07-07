-- 0011_layout_default_enabled — the default-vs-custom layout distinction (#151).
--
-- `is_default` marks the Form/List/Table trio auto-generated with a table: these
-- can be enabled/disabled per view but never deleted. `enabled` gates whether a
-- default view participates in Browse navigation (the sidebar picker + view
-- toggle). Custom layouts (created via the Layout Manager's "New layout") carry
-- `is_default = 0` and are always treated as enabled.
--
-- APPEND-ONLY once shipped (ADR-0004): evolve via new migrations, never by
-- editing this file.

ALTER TABLE meta_layout ADD COLUMN is_default INTEGER NOT NULL DEFAULT 0;
ALTER TABLE meta_layout ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1;
