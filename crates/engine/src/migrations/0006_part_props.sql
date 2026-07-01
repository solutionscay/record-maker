-- 0006_part_props — an appearance bag for layout parts/bands (#49, Issue 7).
-- Parts gain the same opaque `props` JSON slot objects already carry, so a band
-- can hold its own appearance (today: a background `fill`). The server re-derives
-- the band's inline style from these keys on the next read, exactly as it does for
-- objects — Browse and the canvas stay byte-identical (the #44 parity contract).
--
-- No data backfill: record-maker has no legacy layouts to upgrade, so the column
-- lands NULL on any existing row (an unstyled band) and fresh layouts use it
-- directly. A `field`-less structural slot — no other column change.
--
-- APPEND-ONLY once shipped (ADR-0004): evolve via new migrations, never by editing.

ALTER TABLE meta_part ADD COLUMN props TEXT;
