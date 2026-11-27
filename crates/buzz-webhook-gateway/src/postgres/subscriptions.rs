//! Postgres half of [`SubscriptionStore`]: subscription writes, atomic
//! fan-out, and the message outbox.
use super::PostgresStore;
use crate::store::{
    initial_message_state, storable, write_transition, ClaimedMessage, DeliveryState, Evaluated,
    FanoutCommit, FanoutOutcome, ForwardOutcome, MessageKey, StoreError, Subscription,
    SubscriptionChange, SubscriptionStore, SubscriptionWrite, WriteOutcome,
};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use sqlx::{postgres::PgRow, Row};
use uuid::Uuid;

const SUBSCRIPTION_COLUMNS: &str =
    "id, integration_id, channel_id, filter, revision, deleted_at, created_at, updated_at";

fn subscription_from_row(row: PgRow) -> Result<Subscription, StoreError> {
    Ok(Subscription {
        id: row.try_get("id")?,
        integration_id: row.try_get("integration_id")?,
        channel_id: row.try_get("channel_id")?,
        filter: row.try_get("filter")?,
        revision: row.try_get("revision")?,
        deleted_at: row.try_get("deleted_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

#[async_trait]
impl SubscriptionStore for PostgresStore {
    async fn write_subscription(
        &self,
        write: SubscriptionWrite,
        now: DateTime<Utc>,
    ) -> Result<WriteOutcome, StoreError> {
        let mut tx = self.pool.begin().await?;
        // The integration row lock serializes every subscription write of one
        // integration, so the live count below cannot race past the cap.
        let integration_exists =
            sqlx::query("SELECT 1 FROM webhook_gateway_integrations WHERE id = $1 FOR UPDATE")
                .bind(write.integration_id)
                .fetch_optional(&mut *tx)
                .await?
                .is_some();
        if !integration_exists {
            tx.rollback().await?;
            return Ok(WriteOutcome::NotFound);
        }
        let current = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {SUBSCRIPTION_COLUMNS} FROM webhook_gateway_subscriptions
             WHERE id = $1 FOR UPDATE"
        )))
        .bind(write.id)
        .fetch_optional(&mut *tx)
        .await?
        .map(subscription_from_row)
        .transpose()?;
        let live: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM webhook_gateway_subscriptions
             WHERE integration_id = $1 AND deleted_at IS NULL",
        )
        .bind(write.integration_id)
        .fetch_one(&mut *tx)
        .await?;
        let live = usize::try_from(live).unwrap_or(usize::MAX);
        let next = match write_transition(current, &write, live, now) {
            WriteOutcome::Applied(next) => next,
            outcome => {
                tx.rollback().await?;
                return Ok(outcome);
            }
        };
        // A concurrent first write of this id under another integration is
        // not serialized by the integration lock; the guarded upsert refuses
        // to move the row instead.
        let written = sqlx::query(
            "INSERT INTO webhook_gateway_subscriptions
                 (id, integration_id, channel_id, filter, revision, deleted_at, created_at,
                  updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (id) DO UPDATE SET
                 channel_id = EXCLUDED.channel_id,
                 filter = EXCLUDED.filter,
                 revision = EXCLUDED.revision,
                 deleted_at = EXCLUDED.deleted_at,
                 created_at = EXCLUDED.created_at,
                 updated_at = EXCLUDED.updated_at
             WHERE webhook_gateway_subscriptions.integration_id = EXCLUDED.integration_id",
        )
        .bind(next.id)
        .bind(next.integration_id)
        .bind(next.channel_id)
        .bind(&next.filter)
        .bind(next.revision)
        .bind(next.deleted_at)
        .bind(next.created_at)
        .bind(next.updated_at)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if written == 0 {
            tx.rollback().await?;
            return Ok(WriteOutcome::WrongIntegration);
        }
        if matches!(write.change, SubscriptionChange::Delete) {
            sqlx::query(
                "UPDATE webhook_gateway_messages
                 SET state = 'failed', completed_at = $2, next_attempt_at = NULL,
                     last_error = 'subscription deleted'
                 WHERE subscription_id = $1 AND state = 'queued'",
            )
            .bind(next.id)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(WriteOutcome::Applied(next))
    }

    async fn subscription(
        &self,
        integration_id: Uuid,
        id: Uuid,
    ) -> Result<Option<Subscription>, StoreError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {SUBSCRIPTION_COLUMNS} FROM webhook_gateway_subscriptions
             WHERE id = $1 AND integration_id = $2"
        )))
        .bind(id)
        .bind(integration_id)
        .fetch_optional(&self.pool)
        .await?
        .map(subscription_from_row)
        .transpose()
    }

    async fn live_subscriptions(
        &self,
        integration_id: Uuid,
    ) -> Result<Vec<Subscription>, StoreError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {SUBSCRIPTION_COLUMNS} FROM webhook_gateway_subscriptions
             WHERE integration_id = $1 AND deleted_at IS NULL
             ORDER BY created_at, id"
        )))
        .bind(integration_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(subscription_from_row)
        .collect()
    }

    async fn complete_fanout(
        &self,
        integration_id: Uuid,
        delivery_id: &str,
        attempts: i32,
        outcome: FanoutOutcome,
        now: DateTime<Utc>,
    ) -> Result<FanoutCommit, StoreError> {
        let mut tx = self.pool.begin().await?;
        let (state, error, messages) = match outcome {
            FanoutOutcome::Fanned {
                messages,
                evaluated,
            } => {
                // A share lock on the integration row holds off subscription
                // writes, which take it for update, until this commits.
                sqlx::query("SELECT 1 FROM webhook_gateway_integrations WHERE id = $1 FOR SHARE")
                    .bind(integration_id)
                    .execute(&mut *tx)
                    .await?;
                let live: Vec<(Uuid, i64)> = sqlx::query_as(
                    "SELECT id, revision FROM webhook_gateway_subscriptions
                     WHERE integration_id = $1 AND deleted_at IS NULL
",
                )
                .bind(integration_id)
                .fetch_all(&mut *tx)
                .await?;
                let mut live: Vec<_> = live
                    .into_iter()
                    .map(|(id, revision)| Evaluated { id, revision })
                    .collect();
                live.sort();
                if live != evaluated {
                    let due = sqlx::query(
                        "UPDATE webhook_gateway_deliveries SET next_attempt_at = $4
                         WHERE integration_id = $1 AND delivery_id = $2
                           AND state = 'queued' AND attempts = $3",
                    )
                    .bind(integration_id)
                    .bind(delivery_id)
                    .bind(attempts)
                    .bind(now)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
                    tx.commit().await?;
                    return Ok(if due == 0 {
                        FanoutCommit::Lost
                    } else {
                        FanoutCommit::Stale
                    });
                }
                (DeliveryState::Accepted, None, messages)
            }
            FanoutOutcome::Failed(error) => {
                (DeliveryState::Failed, Some(storable(&error)), Vec::new())
            }
        };
        let received_at: Option<DateTime<Utc>> = sqlx::query_scalar(
            "UPDATE webhook_gateway_deliveries
             SET state = $3, completed_at = $4, next_attempt_at = NULL, last_error = $5,
                 body = CASE WHEN $3 = 'accepted' THEN ''::bytea ELSE body END
             WHERE integration_id = $1 AND delivery_id = $2
               AND state = 'queued' AND attempts = $6
             RETURNING received_at",
        )
        .bind(integration_id)
        .bind(delivery_id)
        .bind(state.as_str())
        .bind(now)
        .bind(error)
        .bind(attempts)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(received_at) = received_at else {
            tx.rollback().await?;
            return Ok(FanoutCommit::Lost);
        };
        for message in &messages {
            let (state, content, error) = initial_message_state(message);
            let queued = state == DeliveryState::Queued;
            sqlx::query(
                "INSERT INTO webhook_gateway_messages
                     (integration_id, delivery_id, subscription_id, channel_id, content, state,
                      next_attempt_at, last_error, received_at, completed_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                 ON CONFLICT (integration_id, delivery_id, subscription_id) DO NOTHING",
            )
            .bind(integration_id)
            .bind(delivery_id)
            .bind(message.subscription_id)
            .bind(message.channel_id)
            .bind(content)
            .bind(state.as_str())
            .bind(queued.then_some(received_at))
            .bind(error)
            .bind(received_at)
            .bind((!queued).then_some(now))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(FanoutCommit::Settled)
    }

    async fn claim_due_messages(
        &self,
        now: DateTime<Utc>,
        lease: Duration,
        limit: usize,
    ) -> Result<Vec<ClaimedMessage>, StoreError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        sqlx::query(
            "WITH due AS (
                 SELECT integration_id, delivery_id, subscription_id
                 FROM webhook_gateway_messages
                 WHERE state = 'queued' AND next_attempt_at <= $1
                 ORDER BY next_attempt_at
                 LIMIT $3
                 FOR UPDATE SKIP LOCKED
             ), claimed AS (
                 UPDATE webhook_gateway_messages m
                 SET attempts = m.attempts + 1, next_attempt_at = $2,
                     signed_at = COALESCE(m.signed_at, $1)
                 FROM due
                 WHERE m.integration_id = due.integration_id
                   AND m.delivery_id = due.delivery_id
                   AND m.subscription_id = due.subscription_id
                 RETURNING m.integration_id, m.delivery_id, m.subscription_id, m.channel_id,
                           m.content, m.received_at, m.attempts, m.signed_at, m.event_id
             )
             SELECT c.*, i.community_host
             FROM claimed c
             JOIN webhook_gateway_integrations i ON i.id = c.integration_id",
        )
        .bind(now)
        .bind(now + lease)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| {
            Ok(ClaimedMessage {
                key: MessageKey {
                    integration_id: row.try_get("integration_id")?,
                    delivery_id: row.try_get("delivery_id")?,
                    subscription_id: row.try_get("subscription_id")?,
                },
                community_host: row.try_get("community_host")?,
                channel_id: row.try_get("channel_id")?,
                content: row.try_get("content")?,
                received_at: row.try_get("received_at")?,
                attempts: row.try_get("attempts")?,
                signed_at: row.try_get("signed_at")?,
                event_id: row.try_get("event_id")?,
            })
        })
        .collect()
    }

    async fn complete_message(
        &self,
        message: &ClaimedMessage,
        outcome: ForwardOutcome,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let (state, event_id, error, next_attempt_at, restamp) = match outcome {
            ForwardOutcome::Accepted { event_id } => {
                (DeliveryState::Accepted, Some(event_id), None, None, false)
            }
            ForwardOutcome::Failed { event_id, error } => {
                (DeliveryState::Failed, event_id, Some(error), None, false)
            }
            ForwardOutcome::Retry {
                event_id,
                error,
                next_attempt_at,
            } => (
                DeliveryState::Queued,
                event_id,
                Some(error),
                Some(next_attempt_at),
                false,
            ),
            ForwardOutcome::Restamp { error } => {
                (DeliveryState::Queued, None, Some(error), Some(now), true)
            }
        };
        let terminal = state != DeliveryState::Queued;
        sqlx::query(
            "UPDATE webhook_gateway_messages
             SET state = $5,
                 event_id = CASE WHEN $10 THEN NULL ELSE COALESCE($6, event_id) END,
                 signed_at = CASE WHEN $10 THEN NULL ELSE signed_at END,
                 last_error = $7, next_attempt_at = $8, completed_at = $9
             WHERE integration_id = $1 AND delivery_id = $2 AND subscription_id = $3
               AND state = 'queued' AND attempts = $4",
        )
        .bind(message.key.integration_id)
        .bind(&message.key.delivery_id)
        .bind(message.key.subscription_id)
        .bind(message.attempts)
        .bind(state.as_str())
        .bind(event_id)
        .bind(error.as_deref().map(storable))
        .bind(next_attempt_at)
        .bind(terminal.then_some(now))
        .bind(restamp)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::config::RelayScheme;
    use crate::fanout::Fanout;
    use crate::forwarder::Forwarder;
    use crate::relay_client::{LookupResult, RelayClient, SubmitResult};
    use crate::signer::GatewayKeySigner;
    use crate::store::{
        DeliveryMode, DeliveryPage, IntegrationConfig, Mutation, NewDelivery, Operation, Provider,
        Store,
    };
    use nostr::{Event, Keys, NostrSigner, Url};
    use std::sync::{Arc, Mutex};

    struct AcceptingRelay(Mutex<Vec<(Url, Event)>>);

    #[async_trait]
    impl RelayClient for AcceptingRelay {
        async fn submit(&self, url: &Url, event: &Event, _: &Arc<dyn NostrSigner>) -> SubmitResult {
            self.0.lock().unwrap().push((url.clone(), event.clone()));
            SubmitResult::Accepted
        }

        async fn find(
            &self,
            _: &Url,
            _: &[String],
            _: Uuid,
            _: &Arc<dyn NostrSigner>,
        ) -> LookupResult {
            LookupResult::Missing
        }
    }

    async fn message_row(
        store: &PostgresStore,
        key: &MessageKey,
    ) -> (String, Option<String>, Option<String>, String) {
        sqlx::query_as(
            "SELECT state, event_id, last_error, content FROM webhook_gateway_messages
             WHERE integration_id = $1 AND delivery_id = $2 AND subscription_id = $3",
        )
        .bind(key.integration_id)
        .bind(&key.delivery_id)
        .bind(key.subscription_id)
        .fetch_one(store.pool())
        .await
        .unwrap()
    }

    fn put_sub(
        id: Uuid,
        integration_id: Uuid,
        revision: i64,
        channel: Uuid,
        filter: Option<&str>,
    ) -> SubscriptionWrite {
        SubscriptionWrite {
            id,
            integration_id,
            revision,
            change: SubscriptionChange::Put {
                channel_id: channel,
                filter: filter.map(str::to_owned),
            },
        }
    }

    /// Runs against a dedicated, disposable database: it applies the gateway
    /// migrations and claims every due row in it.
    #[tokio::test]
    #[ignore = "requires Postgres: set BUZZ_TEST_DATABASE_URL to a scratch database"]
    async fn postgres_store_fans_out_sends_and_cancels_messages() {
        let url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .expect("BUZZ_TEST_DATABASE_URL names a scratch database");
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        PostgresStore::migrate_scratch(&pool).await.unwrap();
        let store = Arc::new(PostgresStore::new(pool));
        let now = Utc::now();
        let (integration_id, other_integration) = (Uuid::new_v4(), Uuid::new_v4());
        let configured = IntegrationConfig {
            delivery_mode: DeliveryMode::CallerSelectsChannel,
            channel_expr: Some("payload.channel_id".to_owned()),
            message_expr: Some("payload.title".to_owned()),
            message_data_expr: Some("{\"status\": payload.action}".to_owned()),
            group_key_expr: Some("string(payload.number)".to_owned()),
        };
        for (id, config) in [
            (integration_id, IntegrationConfig::default()),
            (other_integration, configured.clone()),
        ] {
            let operation = Operation {
                operation_id: Uuid::new_v4(),
                integration_id: id,
                revision: 1,
                mutation: Mutation::PutIntegration {
                    community_host: "acme.communities.example.org".to_owned(),
                    provider: Provider::Github,
                    config,
                },
            };
            store.apply(operation, now).await.unwrap();
        }
        let stored = store.integration(other_integration).await.unwrap().unwrap();
        assert_eq!(stored.config, configured, "every config column round-trips");

        let (all, closed, channel) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let write = put_sub(all, integration_id, 1, channel, None);
        assert!(matches!(
            store.write_subscription(write.clone(), now).await.unwrap(),
            WriteOutcome::Applied(_)
        ));
        assert!(matches!(
            store.write_subscription(write, now).await.unwrap(),
            WriteOutcome::Unchanged(_)
        ));
        assert_eq!(
            store
                .write_subscription(put_sub(all, integration_id, 1, Uuid::new_v4(), None), now)
                .await
                .unwrap(),
            WriteOutcome::StaleRevision { current: 1 }
        );
        assert_eq!(
            store
                .write_subscription(put_sub(all, other_integration, 2, channel, None), now)
                .await
                .unwrap(),
            WriteOutcome::WrongIntegration
        );
        assert_eq!(
            store
                .write_subscription(
                    put_sub(Uuid::new_v4(), Uuid::new_v4(), 1, channel, None),
                    now
                )
                .await
                .unwrap(),
            WriteOutcome::NotFound
        );
        let filtered = put_sub(
            closed,
            integration_id,
            1,
            channel,
            Some(r#"payload.action == "closed""#),
        );
        store.write_subscription(filtered, now).await.unwrap();
        assert_eq!(
            store
                .live_subscriptions(integration_id)
                .await
                .unwrap()
                .len(),
            2
        );

        for (delivery_id, received) in [
            ("d-1", now - Duration::seconds(10)),
            ("d-2", now - Duration::seconds(5)),
        ] {
            let delivery = NewDelivery {
                integration_id,
                delivery_id: delivery_id.to_owned(),
                event_type: "pull_request.opened".to_owned(),
                provider_timestamp: None,
                headers: serde_json::json!({}),
                body: br#"{"action":"opened"}"#.to_vec(),
                received_at: received,
            };
            store.insert_delivery(delivery).await.unwrap();
        }
        let fanout = Fanout {
            store: store.clone(),
            concurrency: 64,
            retry_max_age: Duration::hours(1),
            now: Utc::now,
        };
        while fanout.run_once().await.unwrap() > 0 {}
        let key = |delivery_id: &str, subscription_id| MessageKey {
            integration_id,
            delivery_id: delivery_id.to_owned(),
            subscription_id,
        };
        let (state, _, _, content) = message_row(&store, &key("d-1", all)).await;
        assert_eq!(
            (state.as_str(), content.as_str()),
            ("queued", "github: pull_request.opened")
        );
        let unmatched: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM webhook_gateway_messages WHERE subscription_id = $1",
        )
        .bind(closed)
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(unmatched, 0, "the filter kept the closed subscription out");
        let (delivery_state, body_len): (String, i32) = sqlx::query_as(
            "SELECT state, octet_length(body) FROM webhook_gateway_deliveries
             WHERE integration_id = $1 AND delivery_id = 'd-1'",
        )
        .bind(integration_id)
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!((delivery_state.as_str(), body_len), ("accepted", 0));

        // Send the oldest message, then delete the subscription before the
        // other one goes.
        let relay = Arc::new(AcceptingRelay(Mutex::default()));
        let forwarder = Forwarder {
            store: store.clone(),
            client: relay.clone(),
            signer: Arc::new(GatewayKeySigner::new(Keys::generate())),
            scheme: RelayScheme::Https,
            concurrency: 1,
            retry_max_age: Duration::hours(1),
            now: Utc::now,
        };
        assert_eq!(forwarder.run_once().await.unwrap(), 1);
        let (url, event) = relay.0.lock().unwrap()[0].clone();
        assert_eq!(url.as_str(), "https://acme.communities.example.org/events");
        let (state, event_id, _, _) = message_row(&store, &key("d-1", all)).await;
        assert_eq!(state, "accepted");
        assert_eq!(event_id, Some(event.id.to_hex()));
        let timeline = store
            .list_deliveries(
                integration_id,
                DeliveryPage {
                    limit: 10,
                    ..DeliveryPage::default()
                },
            )
            .await
            .unwrap();
        let counts: Vec<_> = timeline
            .iter()
            .map(|d| {
                (
                    d.delivery_id.as_str(),
                    d.messages.queued,
                    d.messages.accepted,
                )
            })
            .collect();
        assert_eq!(counts, [("d-2", 1, 0), ("d-1", 0, 1)]);

        // A fan-out that evaluated a set of subscriptions that has changed
        // since, or whose claim was taken over, stores nothing.
        let late = NewDelivery {
            integration_id,
            delivery_id: "d-3".to_owned(),
            event_type: "pull_request.opened".to_owned(),
            provider_timestamp: None,
            headers: serde_json::json!({}),
            body: br#"{"action":"opened"}"#.to_vec(),
            received_at: now,
        };
        store.insert_delivery(late).await.unwrap();
        let claim = store
            .claim_due(Utc::now(), Duration::seconds(60), 1)
            .await
            .unwrap()
            .remove(0);
        let live = Evaluated::of(&store.live_subscriptions(integration_id).await.unwrap());
        let fanned = |evaluated: Vec<Evaluated>| FanoutOutcome::Fanned {
            messages: vec![crate::store::NewMessage {
                subscription_id: all,
                channel_id: channel,
                content: Ok("late".to_owned()),
            }],
            evaluated,
        };
        for (evaluated, attempts, commit) in [
            (live[..1].to_vec(), claim.attempts, FanoutCommit::Stale),
            (live.clone(), claim.attempts - 1, FanoutCommit::Lost),
        ] {
            assert_eq!(
                store
                    .complete_fanout(integration_id, "d-3", attempts, fanned(evaluated), now)
                    .await
                    .unwrap(),
                commit
            );
        }
        let stored: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM webhook_gateway_messages WHERE delivery_id = 'd-3'",
        )
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(stored, 0);
        assert_eq!(
            store
                .complete_fanout(integration_id, "d-3", claim.attempts, fanned(live), now)
                .await
                .unwrap(),
            FanoutCommit::Settled
        );

        let delete = SubscriptionWrite {
            id: all,
            integration_id,
            revision: 2,
            change: SubscriptionChange::Delete,
        };
        assert!(
            matches!(store.write_subscription(delete, Utc::now()).await.unwrap(), WriteOutcome::Applied(s) if !s.is_live())
        );
        let (state, _, error, _) = message_row(&store, &key("d-2", all)).await;
        assert_eq!(
            (state.as_str(), error.as_deref()),
            ("failed", Some("subscription deleted"))
        );
        assert_eq!(
            store
                .write_subscription(put_sub(all, integration_id, 2, channel, None), now)
                .await
                .unwrap(),
            WriteOutcome::StaleRevision { current: 2 }
        );
        assert!(store
            .subscription(integration_id, all)
            .await
            .unwrap()
            .is_some_and(|s| !s.is_live()));

        store
            .reap_expired(Utc::now() + Duration::days(30), 14)
            .await
            .unwrap();
        assert!(
            store
                .subscription(integration_id, all)
                .await
                .unwrap()
                .is_none(),
            "tombstone reaped"
        );
        let left: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM webhook_gateway_messages WHERE integration_id = $1",
        )
        .bind(integration_id)
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(left, 0, "terminal messages reaped");
    }
}
