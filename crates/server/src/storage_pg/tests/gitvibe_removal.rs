async fn legacy_topic_fixture() -> Option<TestPgStore> {
    let pg = TestPgStore::create().await?;
    pg.store
        .client
        .batch_execute(SCHEMA_MIGRATIONS_SQL)
        .await
        .unwrap();
    for migration in MIGRATIONS.iter().filter(|migration| migration.version < 9) {
        pg.store
            .apply_pending_migration(migration, &migration_checksum(migration.sql))
            .await
            .unwrap();
    }
    for uid in [901, 902] {
        pg.store
            .claim_message(&uuid::Uuid::new_v4().to_string(), &message(uid))
            .await
            .unwrap();
    }
    pg.store.client.batch_execute(
        "INSERT INTO email_rules (mailbox_id, name, category_id, action, reply_goal, enabled)
         SELECT 'support', names.name, c.id, 'reply', 'preserve operator goal', names.enabled
         FROM email_categories c CROSS JOIN (VALUES
             ('retired-only', TRUE), ('mixed-topics', TRUE), ('already-disabled', FALSE), ('unaffected', TRUE)
         ) AS names(name, enabled) WHERE c.name = 'question';
         INSERT INTO email_rule_topics (rule_id, topic_id)
         SELECT r.id, t.id FROM email_rules r CROSS JOIN email_topics t
         WHERE (r.name IN ('retired-only', 'mixed-topics', 'already-disabled') AND t.name = 'gitvibe')
            OR (r.name IN ('mixed-topics', 'unaffected') AND t.name = 'general');
         UPDATE processing_runs SET classification_topic_ids = ARRAY(
             SELECT id FROM email_topics WHERE name IN ('gitvibe', 'general') ORDER BY id
         ), classification_reason = 'preserve historical reason', classification_confidence = 99,
            matched_rule_name = 'preserve rule snapshot', matched_rule_goal = 'preserve historical goal'
         WHERE uid = 901;
         UPDATE processing_runs SET classification_topic_ids = ARRAY(
             SELECT id FROM email_topics WHERE name = 'general'
         ) WHERE uid = 902;"
    ).await.unwrap();
    Some(pg)
}

async fn message_snapshots(store: &PgStore) -> Vec<String> {
    store
        .client
        .query(
            "SELECT (to_jsonb(pr) - 'classification_topic_ids')::text
        FROM processing_runs pr ORDER BY uid",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect()
}

#[test]
fn pg_store_upgrade_removes_topic_without_broadening_rules_or_changing_messages() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (ran, events) = crate::logging::test_capture::capture(|| {
        runtime.block_on(async {
            let Some(pg) = legacy_topic_fixture().await else {
                return false;
            };
            let before = message_snapshots(&pg.store).await;
            pg.store.migrate().await.unwrap();
            pg.store.migrate().await.unwrap();
            assert_eq!(message_snapshots(&pg.store).await, before);
            pg.store
                .ensure_default_classification_policy(&app_config_with_mailboxes(vec!["support"]))
                .await
                .unwrap();
            let config = pg.store.list_email_classification_config().await.unwrap();
            assert!(!config.topics.iter().any(|topic| topic.name == "gitvibe"));
            for name in ["retired-only", "mixed-topics", "already-disabled"] {
                let rule = config.rules.iter().find(|rule| rule.name == name).unwrap();
                assert!(!rule.enabled, "dependent rule was broadened");
                assert_eq!(rule.reply_goal, "preserve operator goal");
                assert!(!rule.topics.iter().any(|topic| topic == "gitvibe"));
            }
            let mixed = config
                .rules
                .iter()
                .find(|rule| rule.name == "mixed-topics")
                .unwrap();
            assert_eq!(mixed.topics, vec!["general"]);
            assert!(
                config
                    .rules
                    .iter()
                    .find(|rule| rule.name == "unaffected")
                    .unwrap()
                    .enabled
            );
            let general: i64 = pg
                .store
                .client
                .query_one("SELECT id FROM email_topics WHERE name = 'general'", &[])
                .await
                .unwrap()
                .get(0);
            let rows = pg
                .store
                .client
                .query(
                    "SELECT classification_topic_ids FROM processing_runs ORDER BY uid",
                    &[],
                )
                .await
                .unwrap();
            assert_eq!(rows.len(), 2);
            assert!(rows
                .iter()
                .all(|row| row.get::<_, Vec<i64>>(0) == vec![general]));
            let count: i64 = pg
                .store
                .client
                .query_one(
                    "SELECT count(*) FROM schema_migrations WHERE version = 9",
                    &[],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(count, 1);
            pg.cleanup().await;
            true
        })
    });
    if !ran {
        return;
    }
    let migration = events
        .iter()
        .find(|event| {
            event["span"]["migration_version"] == 9 && event["fields"]["status"] == "applied"
        })
        .unwrap();
    assert_eq!(migration["level"], "INFO");
    assert_eq!(migration["fields"]["affected_rows"], "[2, 1, 1]");
    assert!(migration["span"]["checksum"]
        .as_str()
        .is_some_and(|checksum| checksum.len() == 64));
}

#[test]
fn pg_store_failed_topic_removal_rolls_back_and_logs_the_cause() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (ran, events) =
        crate::logging::test_capture::capture(|| {
            runtime.block_on(async {
                let Some(pg) = legacy_topic_fixture().await else {
                    return false;
                };
                let before = message_snapshots(&pg.store).await;
                pg.store.client.batch_execute(
            "CREATE FUNCTION reject_topic_removal() RETURNS trigger LANGUAGE plpgsql AS $$
             BEGIN RAISE EXCEPTION 'synthetic migration constraint'; END $$;
             CREATE TRIGGER reject_topic_removal BEFORE DELETE ON email_topics
             FOR EACH ROW WHEN (OLD.name = 'gitvibe') EXECUTE FUNCTION reject_topic_removal();"
        ).await.unwrap();
                assert!(pg.store.migrate().await.is_err());
                assert_eq!(message_snapshots(&pg.store).await, before);
                let active: bool = pg
                    .store
                    .client
                    .query_one(
                        "SELECT enabled FROM email_rules WHERE name = 'retired-only'",
                        &[],
                    )
                    .await
                    .unwrap()
                    .get(0);
                assert!(active);
                let present: i64 = pg
                    .store
                    .client
                    .query_one(
                        "SELECT count(*) FROM email_topics WHERE name = 'gitvibe'",
                        &[],
                    )
                    .await
                    .unwrap()
                    .get(0);
                assert_eq!(present, 1);
                let applied: i64 = pg
                    .store
                    .client
                    .query_one(
                        "SELECT count(*) FROM schema_migrations WHERE version = 9",
                        &[],
                    )
                    .await
                    .unwrap()
                    .get(0);
                assert_eq!(applied, 0);
                pg.store
                    .client
                    .batch_execute("DROP TRIGGER reject_topic_removal ON email_topics")
                    .await
                    .unwrap();
                pg.store.migrate().await.unwrap();
                pg.cleanup().await;
                true
            })
        });
    if !ran {
        return;
    }
    let failure = events
        .iter()
        .find(|event| {
            event["span"]["migration_version"] == 9 && event["fields"]["status"] == "rolled_back"
        })
        .unwrap();
    assert_eq!(failure["level"], "ERROR");
    assert!(failure["fields"]["cause"]
        .as_str()
        .unwrap()
        .contains("synthetic migration constraint"));
    assert!(events
        .iter()
        .any(|event| event["span"]["migration_version"] == 9
            && event["fields"]["status"] == "applied"));
}
