impl PgStore {
    pub async fn list_email_classification_config(
        &self,
    ) -> Result<EmailClassificationConfig, StorageError> {
        Ok(EmailClassificationConfig {
            categories: self.list_email_categories().await?,
            topics: self.list_email_topics().await?,
            rules: self.list_email_rules().await?,
        })
    }

    async fn list_email_categories(&self) -> Result<Vec<EmailCategory>, StorageError> {
        let rows = self
            .client
            .query(
                "SELECT id, name, description, status, source, created_at::text, updated_at::text
                FROM email_categories
                ORDER BY name ASC",
                &[],
            )
            .await?;
        Ok(rows.into_iter().map(email_category_from_row).collect())
    }

    async fn list_email_topics(&self) -> Result<Vec<EmailTopic>, StorageError> {
        let rows = self
            .client
            .query(
                "SELECT id, name, description, status, source, created_at::text, updated_at::text
                FROM email_topics
                ORDER BY name ASC",
                &[],
            )
            .await?;
        Ok(rows.into_iter().map(email_topic_from_row).collect())
    }

    async fn list_email_rules(&self) -> Result<Vec<EmailRule>, StorageError> {
        let rows = self
            .client
            .query(
                "SELECT r.id, r.mailbox_id, r.name, r.category_id, c.name,
                    COALESCE(array_remove(array_agg(rt.topic_id ORDER BY t.name), NULL), '{}'),
                    COALESCE(array_remove(array_agg(t.name ORDER BY t.name), NULL), '{}'),
                    r.action, r.reply_goal, r.enabled, r.priority, r.created_at::text, r.updated_at::text
                FROM email_rules r
                JOIN email_categories c ON c.id = r.category_id
                LEFT JOIN email_rule_topics rt ON rt.rule_id = r.id
                LEFT JOIN email_topics t ON t.id = rt.topic_id
                GROUP BY r.id, c.name
                ORDER BY r.mailbox_id ASC, r.priority ASC, r.id ASC",
                &[],
            )
            .await?;
        rows.into_iter().map(email_rule_from_row).collect()
    }

    pub async fn create_email_category(
        &self,
        name: &str,
        description: &str,
    ) -> Result<EmailCategory, StorageError> {
        let name = normalize_label_name(name);
        let row = self
            .client
            .query_one(
                "INSERT INTO email_categories (name, description, source)
                VALUES ($1, $2, 'user')
                ON CONFLICT (name) DO UPDATE
                SET description = EXCLUDED.description,
                    status = 'active',
                    updated_at = now()
                RETURNING id, name, description, status, source, created_at::text, updated_at::text",
                &[&name, &description],
            )
            .await?;
        Ok(email_category_from_row(row))
    }

    pub async fn update_email_category(
        &self,
        id: i64,
        name: &str,
        description: &str,
        status: &str,
    ) -> Result<EmailCategory, StorageError> {
        validate_label_status(status)?;
        let name = normalize_label_name(name);
        let row = self
            .client
            .query_opt(
                "UPDATE email_categories
                SET name = $1, description = $2, status = $3, updated_at = now()
                WHERE id = $4
                RETURNING id, name, description, status, source, created_at::text, updated_at::text",
                &[&name, &description, &status, &id],
            )
            .await?
            .ok_or_else(|| StorageError::ClassificationNotFound(format!("category {id}")))?;
        Ok(email_category_from_row(row))
    }

    pub async fn delete_email_category(&self, id: i64) -> Result<(), StorageError> {
        let deleted = self
            .client
            .execute("DELETE FROM email_categories WHERE id = $1", &[&id])
            .await?;
        if deleted == 0 {
            return Err(StorageError::ClassificationNotFound(format!(
                "category {id}"
            )));
        }
        Ok(())
    }

    pub async fn merge_email_category(
        &self,
        source_id: i64,
        target_id: i64,
    ) -> Result<(), StorageError> {
        if source_id == target_id {
            return Err(StorageError::InvalidClassification(
                "source and target category must differ".to_string(),
            ));
        }
        self.client.batch_execute("BEGIN").await?;
        let result: Result<(), StorageError> = async {
            self.require_category(target_id).await?;
            self.require_category(source_id).await?;
            self.client
                .execute(
                    "UPDATE processing_runs
                    SET classification_category_id = $1, updated_at = now()
                    WHERE classification_category_id = $2",
                    &[&target_id, &source_id],
                )
                .await?;
            self.client
                .execute(
                    "UPDATE email_rules
                    SET category_id = $1, updated_at = now()
                    WHERE category_id = $2",
                    &[&target_id, &source_id],
                )
                .await?;
            self.client
                .execute("DELETE FROM email_categories WHERE id = $1", &[&source_id])
                .await?;
            Ok(())
        }
        .await;
        self.finish_transaction(result).await
    }

    pub async fn create_email_topic(
        &self,
        name: &str,
        description: &str,
    ) -> Result<EmailTopic, StorageError> {
        let name = normalize_label_name(name);
        let row = self
            .client
            .query_one(
                "INSERT INTO email_topics (name, description, source)
                VALUES ($1, $2, 'user')
                ON CONFLICT (name) DO UPDATE
                SET description = EXCLUDED.description,
                    status = 'active',
                    updated_at = now()
                RETURNING id, name, description, status, source, created_at::text, updated_at::text",
                &[&name, &description],
            )
            .await?;
        Ok(email_topic_from_row(row))
    }

    pub async fn update_email_topic(
        &self,
        id: i64,
        name: &str,
        description: &str,
        status: &str,
    ) -> Result<EmailTopic, StorageError> {
        validate_label_status(status)?;
        let name = normalize_label_name(name);
        let row = self
            .client
            .query_opt(
                "UPDATE email_topics
                SET name = $1, description = $2, status = $3, updated_at = now()
                WHERE id = $4
                RETURNING id, name, description, status, source, created_at::text, updated_at::text",
                &[&name, &description, &status, &id],
            )
            .await?
            .ok_or_else(|| StorageError::ClassificationNotFound(format!("topic {id}")))?;
        Ok(email_topic_from_row(row))
    }

    pub async fn delete_email_topic(&self, id: i64) -> Result<(), StorageError> {
        let deleted = self
            .client
            .execute("DELETE FROM email_topics WHERE id = $1", &[&id])
            .await?;
        if deleted == 0 {
            return Err(StorageError::ClassificationNotFound(format!("topic {id}")));
        }
        Ok(())
    }

    pub async fn merge_email_topic(
        &self,
        source_id: i64,
        target_id: i64,
    ) -> Result<(), StorageError> {
        if source_id == target_id {
            return Err(StorageError::InvalidClassification(
                "source and target topic must differ".to_string(),
            ));
        }
        self.client.batch_execute("BEGIN").await?;
        let result: Result<(), StorageError> = async {
            self.require_topic(target_id).await?;
            self.require_topic(source_id).await?;
            self.client
                .execute(
                    "UPDATE processing_runs
                    SET classification_topic_ids = ARRAY(
                        SELECT DISTINCT CASE WHEN topic_id = $1 THEN $2 ELSE topic_id END
                        FROM unnest(classification_topic_ids) AS topic_id
                    ),
                    updated_at = now()
                    WHERE $1 = ANY(classification_topic_ids)",
                    &[&source_id, &target_id],
                )
                .await?;
            self.client
                .execute(
                    "INSERT INTO email_rule_topics (rule_id, topic_id)
                    SELECT rule_id, $1 FROM email_rule_topics WHERE topic_id = $2
                    ON CONFLICT DO NOTHING",
                    &[&target_id, &source_id],
                )
                .await?;
            self.client
                .execute(
                    "DELETE FROM email_rule_topics WHERE topic_id = $1",
                    &[&source_id],
                )
                .await?;
            self.client
                .execute("DELETE FROM email_topics WHERE id = $1", &[&source_id])
                .await?;
            Ok(())
        }
        .await;
        self.finish_transaction(result).await
    }

    pub async fn create_email_rule(&self, rule: NewEmailRule) -> Result<EmailRule, StorageError> {
        validate_rule(&rule)?;
        self.client.batch_execute("BEGIN").await?;
        let result = self.insert_email_rule(rule).await;
        self.finish_transaction(result).await
    }

    pub async fn update_email_rule(
        &self,
        id: i64,
        rule: NewEmailRule,
    ) -> Result<EmailRule, StorageError> {
        validate_rule(&rule)?;
        self.client.batch_execute("BEGIN").await?;
        let result: Result<EmailRule, StorageError> = async {
            let row = self
                .client
                .query_opt(
                    "UPDATE email_rules
                    SET mailbox_id = $1, name = $2, category_id = $3, action = $4,
                        reply_goal = $5, enabled = $6, priority = $7, updated_at = now()
                    WHERE id = $8
                    RETURNING id",
                    &[
                        &rule.mailbox_id,
                        &rule.name,
                        &rule.category_id,
                        &rule.action.as_str(),
                        &rule.reply_goal,
                        &rule.enabled,
                        &rule.priority,
                        &id,
                    ],
                )
                .await?
                .ok_or_else(|| StorageError::ClassificationNotFound(format!("rule {id}")))?;
            let rule_id: i64 = row.get(0);
            self.replace_rule_topics(rule_id, &rule.topic_ids).await?;
            self.get_email_rule(rule_id).await
        }
        .await;
        self.finish_transaction(result).await
    }

    pub async fn delete_email_rule(&self, id: i64) -> Result<(), StorageError> {
        let deleted = self
            .client
            .execute("DELETE FROM email_rules WHERE id = $1", &[&id])
            .await?;
        if deleted == 0 {
            return Err(StorageError::ClassificationNotFound(format!("rule {id}")));
        }
        Ok(())
    }

    async fn insert_email_rule(&self, rule: NewEmailRule) -> Result<EmailRule, StorageError> {
        let row = self
            .client
            .query_one(
                "INSERT INTO email_rules
                (mailbox_id, name, category_id, action, reply_goal, enabled, priority)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                RETURNING id",
                &[
                    &rule.mailbox_id,
                    &rule.name,
                    &rule.category_id,
                    &rule.action.as_str(),
                    &rule.reply_goal,
                    &rule.enabled,
                    &rule.priority,
                ],
            )
            .await?;
        let rule_id: i64 = row.get(0);
        self.replace_rule_topics(rule_id, &rule.topic_ids).await?;
        self.get_email_rule(rule_id).await
    }

    async fn replace_rule_topics(
        &self,
        rule_id: i64,
        topic_ids: &[i64],
    ) -> Result<(), StorageError> {
        self.client
            .execute(
                "DELETE FROM email_rule_topics WHERE rule_id = $1",
                &[&rule_id],
            )
            .await?;
        for topic_id in topic_ids {
            self.client
                .execute(
                    "INSERT INTO email_rule_topics (rule_id, topic_id)
                    VALUES ($1, $2)
                    ON CONFLICT DO NOTHING",
                    &[&rule_id, topic_id],
                )
                .await?;
        }
        Ok(())
    }

    async fn get_email_rule(&self, id: i64) -> Result<EmailRule, StorageError> {
        let row = self
            .client
            .query_opt(
                "SELECT r.id, r.mailbox_id, r.name, r.category_id, c.name,
                    COALESCE(array_remove(array_agg(rt.topic_id ORDER BY t.name), NULL), '{}'),
                    COALESCE(array_remove(array_agg(t.name ORDER BY t.name), NULL), '{}'),
                    r.action, r.reply_goal, r.enabled, r.priority, r.created_at::text, r.updated_at::text
                FROM email_rules r
                JOIN email_categories c ON c.id = r.category_id
                LEFT JOIN email_rule_topics rt ON rt.rule_id = r.id
                LEFT JOIN email_topics t ON t.id = rt.topic_id
                WHERE r.id = $1
                GROUP BY r.id, c.name",
                &[&id],
            )
            .await?
            .ok_or_else(|| StorageError::ClassificationNotFound(format!("rule {id}")))?;
        email_rule_from_row(row)
    }

    async fn require_category(&self, id: i64) -> Result<(), StorageError> {
        let exists = self
            .client
            .query_opt("SELECT 1 FROM email_categories WHERE id = $1", &[&id])
            .await?
            .is_some();
        if exists {
            Ok(())
        } else {
            Err(StorageError::ClassificationNotFound(format!(
                "category {id}"
            )))
        }
    }

    async fn require_topic(&self, id: i64) -> Result<(), StorageError> {
        let exists = self
            .client
            .query_opt("SELECT 1 FROM email_topics WHERE id = $1", &[&id])
            .await?
            .is_some();
        if exists {
            Ok(())
        } else {
            Err(StorageError::ClassificationNotFound(format!("topic {id}")))
        }
    }

    async fn finish_transaction<T>(
        &self,
        result: Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        match result {
            Ok(value) => {
                self.client.batch_execute("COMMIT").await?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.client.batch_execute("ROLLBACK").await;
                Err(error)
            }
        }
    }

}
