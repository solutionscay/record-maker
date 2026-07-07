-- 0010_layout_position — ordering for the flat Layout Manager list (#149).
-- APPEND-ONLY once shipped (ADR-0004): evolve via new migrations, never by
-- editing this file.

ALTER TABLE meta_layout ADD COLUMN position INTEGER NOT NULL DEFAULT 0;

CREATE INDEX idx_meta_layout_position ON meta_layout(position, id);
