-- Model B portal containment (#168/#169): a portal OWNS its column field objects
-- via a self-FK on meta_object. `parent_object_id` is NULL for a normal top-level
-- object, or the id of the owning portal object for a column placed inside it.
--
-- ON DELETE CASCADE means deleting a portal removes its column children in one
-- shot (foreign_keys is ON for every app.db connection — see db.rs). Nullable
-- with a NULL default so every existing object stays top-level (no data repair).
ALTER TABLE meta_object
    ADD COLUMN parent_object_id INTEGER
        REFERENCES meta_object(id) ON DELETE CASCADE;

CREATE INDEX idx_meta_object_parent ON meta_object(parent_object_id);
