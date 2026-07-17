-- 0017_layout_grid — one editor grid owned by the entire layout (#193).
--
-- Bands keep only their local structure/appearance. Grid geometry belongs to
-- meta_layout because the same coordinate system spans Header, Body, Footer,
-- and summary bands. A 1px default removes the old client-only 8px stepping
-- while retaining integer geometry. Visibility is Layout-mode UI state stored
-- with the solution; Browse never renders the grid.
--
-- APPEND-ONLY once shipped (ADR-0004): evolve via new migrations, never by
-- editing this file.

ALTER TABLE meta_layout ADD COLUMN grid_size INTEGER NOT NULL DEFAULT 1 CHECK (grid_size >= 1);
ALTER TABLE meta_layout ADD COLUMN show_grid INTEGER NOT NULL DEFAULT 1 CHECK (show_grid IN (0, 1));
ALTER TABLE meta_layout ADD COLUMN snap_to_grid INTEGER NOT NULL DEFAULT 1 CHECK (snap_to_grid IN (0, 1));
