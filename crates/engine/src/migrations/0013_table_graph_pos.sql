-- 0013_table_graph_pos — saved x/y of each table box in the Relationships
-- graph, so an arranged diagram survives reloads (#142 follow-up).
-- APPEND-ONLY once shipped (ADR-0004): evolve via new migrations, never by
-- editing this file. Nullable: a table with no saved coords falls back to the
-- computed grid layout until the user first drags it.

ALTER TABLE meta_table ADD COLUMN graph_x REAL;
ALTER TABLE meta_table ADD COLUMN graph_y REAL;
