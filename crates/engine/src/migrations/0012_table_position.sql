-- 0012_table_position — ordering for the flat tables list (#162).
-- APPEND-ONLY once shipped (ADR-0004): evolve via new migrations, never by
-- editing this file.

ALTER TABLE meta_table ADD COLUMN position INTEGER NOT NULL DEFAULT 0;

CREATE INDEX idx_meta_table_position ON meta_table(position, id);
