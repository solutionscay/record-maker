-- 0002_layout_contract — structural Layout-Mode contract for the design canvas (#43).
-- Extends the object model so the editor and engine agree on every property the
-- canvas reads/writes. Two genuinely-new structural properties on objects:
--
--   z          stacking order within a part (overlap resolution). Higher = front.
--   read_only  per-object Browse editability. 1 → Browse renders a non-editable
--              value instead of an input (the per-object behaviour from #40/#43).
--
-- The geometry contract (x/y/w/h relative to the owning part), the object/part
-- KIND sets, and part height/resize semantics are CONFIRMED by 0001 + the engine
-- accessors; they need no column changes here. Appearance/styling (fill, border,
-- fonts, colour) is owned separately by #49 and rides in meta_object.props.
--
-- APPEND-ONLY once shipped (ADR-0004): evolve via new migrations, never by
-- editing this. ADD COLUMN with a NOT NULL DEFAULT backfills existing rows, so
-- every layout authored before this migration stays valid (z=0, editable).

ALTER TABLE meta_object ADD COLUMN z         INTEGER NOT NULL DEFAULT 0;
ALTER TABLE meta_object ADD COLUMN read_only INTEGER NOT NULL DEFAULT 0;
