-- 0005_deduplicate_singleton_parts — enforce one header/body/footer per layout.
--
-- Earlier builds allowed duplicate singleton parts. Keep the first part of each
-- singleton kind by layout order and convert later duplicates to subsummaries so
-- any objects in those bands are preserved rather than deleted.

UPDATE meta_part
SET kind = 'subsummary'
WHERE kind IN ('header', 'body', 'footer')
  AND id NOT IN (
      SELECT keep_id
      FROM (
          SELECT MIN(id) AS keep_id
          FROM meta_part
          WHERE kind IN ('header', 'body', 'footer')
          GROUP BY layout_id, kind
      )
  );
