WITH duplicate_default_rules AS (
    SELECT id
    FROM (
        SELECT
            r.id,
            row_number() OVER (
                PARTITION BY r.mailbox_id, r.category_id, r.name
                ORDER BY r.id
            ) AS duplicate_rank
        FROM email_rules r
        JOIN email_categories c ON c.id = r.category_id
        WHERE c.name = 'marketing_vendor'
          AND r.name = 'Auto-decline marketing/vendor outreach'
    ) ranked
    WHERE duplicate_rank > 1
)
DELETE FROM email_rules
WHERE id IN (SELECT id FROM duplicate_default_rules);

CREATE UNIQUE INDEX IF NOT EXISTS email_rules_default_marketing_seed_unique_idx
ON email_rules (mailbox_id, category_id, name)
WHERE name = 'Auto-decline marketing/vendor outreach';
