-- #184: a portal's stored geometry is its reusable FIRST ROW, not its whole
-- scrolling viewport. `props.rowCount` independently controls how many repeated
-- rows the Layout preview and Browse viewport show.
--
-- Existing portals used `h` as total viewport height while the tallest authored
-- child field supplied the row height. Preserve their visible row capacity by
-- recording floor(old viewport / inferred row height), minimum one, before
-- replacing `h` with that inferred row height. A portal without columns uses a
-- conventional 24px row (or its smaller existing height) as the fallback.
UPDATE meta_object AS portal
SET props = json_set(
    CASE WHEN json_valid(portal.props) THEN portal.props ELSE '{}' END,
    '$.rowCount',
    max(
        1,
        portal.h / max(
            1,
            coalesce(
                (SELECT max(child.h)
                   FROM meta_object AS child
                  WHERE child.parent_object_id = portal.id
                    AND child.kind = 'field'),
                min(portal.h, 24)
            )
        )
    )
)
WHERE portal.kind = 'portal';

UPDATE meta_object AS portal
SET h = max(
    1,
    coalesce(
        (SELECT max(child.h)
           FROM meta_object AS child
          WHERE child.parent_object_id = portal.id
            AND child.kind = 'field'),
        min(portal.h, 24)
    )
)
WHERE portal.kind = 'portal';
