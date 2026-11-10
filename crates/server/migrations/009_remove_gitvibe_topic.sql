UPDATE email_rules r
SET enabled = FALSE, updated_at = now()
WHERE r.enabled = TRUE
  AND EXISTS (
      SELECT 1
      FROM email_rule_topics rt
      JOIN email_topics t ON t.id = rt.topic_id
      WHERE rt.rule_id = r.id AND t.name = 'gitvibe'
  );

UPDATE processing_runs pr
SET classification_topic_ids = array_remove(pr.classification_topic_ids, t.id)
FROM email_topics t
WHERE t.name = 'gitvibe'
  AND t.id = ANY(pr.classification_topic_ids);

DELETE FROM email_topics WHERE name = 'gitvibe';
