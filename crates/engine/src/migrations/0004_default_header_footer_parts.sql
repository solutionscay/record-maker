-- 0004_default_header_footer_parts — default layouts have header/body/footer.
--
-- Earlier default layouts could be body-only. The layout part rules now treat
-- header/body/footer as the normal skeleton for a layout, so backfill existing
-- layouts without creating duplicate singleton parts.
--
-- Header is inserted at the top. Existing parts in layouts missing a header shift
-- down one position so the current body keeps its relative objects/geometry.
UPDATE meta_part
SET position = position + 1
WHERE layout_id IN (
    SELECT l.id
    FROM meta_layout l
    WHERE NOT EXISTS (
        SELECT 1 FROM meta_part p
        WHERE p.layout_id = l.id AND p.kind = 'header'
    )
);

INSERT INTO meta_part(layout_id, kind, height, position)
SELECT l.id, 'header', 40, 0
FROM meta_layout l
WHERE NOT EXISTS (
    SELECT 1 FROM meta_part p
    WHERE p.layout_id = l.id AND p.kind = 'header'
);

-- Footer is inserted after the current bottom part.
INSERT INTO meta_part(layout_id, kind, height, position)
SELECT l.id,
       'footer',
       40,
       COALESCE((SELECT MAX(p.position) + 1 FROM meta_part p WHERE p.layout_id = l.id), 0)
FROM meta_layout l
WHERE NOT EXISTS (
    SELECT 1 FROM meta_part p
    WHERE p.layout_id = l.id AND p.kind = 'footer'
);
