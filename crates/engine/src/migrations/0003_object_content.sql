-- 0003_object_content — a dedicated text slot for label/text objects (#60).
-- The object model is one meta_object discriminated by `kind`; a `text` object
-- carries its literal in its OWN slot rather than overloading `binding` (which is
-- data-paths only). `field` objects leave it NULL; `shape` objects (rect/line/
-- ellipse) leave it NULL too and draw from `props`.
--
-- No data backfill: record-maker has no legacy layouts to upgrade, so the column
-- lands NULL on any existing row and fresh `create_table` output uses it directly
-- (the default form spawns a separate label text object per field). The new shape
-- kinds are a `kind`-set extension only — no column change beyond this one.
--
-- APPEND-ONLY once shipped (ADR-0004): evolve via new migrations, never by editing.

ALTER TABLE meta_object ADD COLUMN content TEXT;
