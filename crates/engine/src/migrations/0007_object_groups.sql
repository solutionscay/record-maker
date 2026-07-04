-- 0007_object_groups -- durable layout-object groups (#75).
--
-- A group is a metadata relationship over existing objects, not a renderable
-- object. Child objects keep their own geometry, z, styles, and part membership;
-- the group only says "select/move these ids as one unit" and persists that
-- relationship across reload.
CREATE TABLE meta_object_group (
    id        INTEGER PRIMARY KEY,
    layout_id INTEGER NOT NULL REFERENCES meta_layout(id) ON DELETE CASCADE
);

CREATE TABLE meta_object_group_member (
    group_id  INTEGER NOT NULL REFERENCES meta_object_group(id) ON DELETE CASCADE,
    object_id INTEGER NOT NULL REFERENCES meta_object(id) ON DELETE CASCADE,
    position  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (group_id, object_id),
    UNIQUE (object_id)
);

CREATE INDEX idx_meta_object_group_layout ON meta_object_group(layout_id);
CREATE INDEX idx_meta_object_group_member_object ON meta_object_group_member(object_id);
