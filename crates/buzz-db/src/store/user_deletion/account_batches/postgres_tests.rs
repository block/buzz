use super::*;
use crate::Db;
use sqlx::PgPool;
use std::collections::BTreeSet;

#[path = "postgres_tests/failures.rs"]
mod failures_postgres_tests;
#[path = "postgres_tests/stage.rs"]
mod stage_postgres_tests;

#[path = "postgres_tests/expansion_postgres_tests.rs"]
mod expansion_postgres_tests;

#[path = "memberships_postgres_tests.rs"]
mod memberships_postgres_tests;

#[path = "community_memberships_postgres_tests.rs"]
mod community_memberships_postgres_tests;

#[path = "ownership_postgres_tests.rs"]
mod ownership_postgres_tests;

const TARGET: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
const TABLES: &[&str] = &[
    "api_tokens",
    "relay_invites",
    "channel_members",
    "join_policy_acceptances",
    "relay_members",
    "pubkey_allowlist",
    "personal_read_frontiers",
    "personal_read_accounts",
    "archived_identities",
    "parameterized_event_watermarks",
    "users",
    "channels",
    "entity_deletion_requests",
    "entity_deletion_stage_history",
    "entity_deletion_approvals",
    "user_deletion_fences",
];

struct Fixture {
    pool: PgPool,
    options: AccountBatchOptions,
    other: Uuid,
}

impl Fixture {
    async fn new() -> Self {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let community = Uuid::new_v4();
        let other = Uuid::new_v4();
        let seed = include_str!("fixture.sql")
            .replace("__TARGET__", TARGET)
            .replace("__COMMUNITY__", &community.to_string())
            .replace("__OTHER__", &other.to_string());
        sqlx::raw_sql(sqlx::AssertSqlSafe(seed))
            .execute(&pool)
            .await
            .unwrap();
        Self {
            pool,
            other,
            options: AccountBatchOptions {
                community,
                pubkey: TARGET.to_owned(),
                batch_size: 2,
                max_items: 1000,
            },
        }
    }

    fn store(&self) -> UserDeletionStore {
        Db::from_pool(self.pool.clone())
            .user_deletion_store()
            .with_account_snapshot_signer(nostr::Keys::parse(&format!("{:064x}", 1)).unwrap())
    }

    async fn run(&self, mode: ExecutionMode, out: &mut impl Write) -> Result<AccountBatchSummary> {
        self.store()
            .run_account_batches(&self.options, mode, out, None)
            .await
    }

    async fn snapshot(&self) -> Value {
        let mut snapshot = serde_json::Map::new();
        for table in TABLES {
            let sql = format!("SELECT COALESCE(jsonb_agg(to_jsonb(r) ORDER BY to_jsonb(r)), '[]'::jsonb) FROM {table} r");
            let rows: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
                .fetch_one(&self.pool)
                .await
                .unwrap();
            snapshot.insert((*table).to_owned(), rows);
        }
        Value::Object(snapshot)
    }

    async fn assert_cleaned(&self, before: &Value, communities: &[Uuid]) {
        let mut expected = before.clone();
        let mut actual = self.snapshot().await;
        for table in TABLES {
            // Independent outcome oracle: ownership removes a row; attribution
            // only loses that value. All other columns and tables must survive.
            let owner = match *table {
                "api_tokens" => Some("owner_pubkey"),
                "relay_invites" => Some("created_by"),
                "channel_members"
                | "join_policy_acceptances"
                | "relay_members"
                | "pubkey_allowlist" => Some("pubkey"),
                "personal_read_frontiers" | "personal_read_accounts" => Some("actor"),
                _ => None,
            };
            let fields: &[&str] = match *table {
                "api_tokens" => &["revoked_by"],
                "channel_members" => &["invited_by", "removed_by"],
                "relay_members" | "pubkey_allowlist" => &["added_by"],
                "archived_identities" => &["replaced_by"],
                _ => &[],
            };
            let in_scope = |row: &Value| {
                communities
                    .iter()
                    .any(|id| row["community_id"] == id.to_string())
            };
            let subject = |v: &Value| v == TARGET || *v == format!("\\x{TARGET}");
            let rows = expected[*table].as_array_mut().unwrap();
            rows.retain(|row| !(in_scope(row) && owner.is_some_and(|key| subject(&row[key]))));
            for row in rows.iter_mut().filter(|row| in_scope(row)) {
                let mut changed = false;
                for key in fields {
                    if subject(&row[*key]) {
                        row[*key] = Value::Null;
                        changed = true;
                    }
                }
                if changed && matches!(*table, "channel_members" | "relay_members") {
                    let timestamp = if *table == "channel_members" {
                        "sync_modified_at"
                    } else {
                        "updated_at"
                    };
                    let retained = actual[*table]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|r| {
                            r["community_id"] == row["community_id"]
                                && r["pubkey"] == row["pubkey"]
                                && r["channel_id"] == row["channel_id"]
                        })
                        .unwrap();
                    assert!(
                        retained[timestamp].as_str().unwrap() > row[timestamp].as_str().unwrap()
                    );
                    retained.as_object_mut().unwrap().remove(timestamp);
                    row.as_object_mut().unwrap().remove(timestamp);
                }
            }
            rows.sort_by_key(Value::to_string);
            actual[*table]
                .as_array_mut()
                .unwrap()
                .sort_by_key(Value::to_string);
        }
        assert_eq!(actual, expected);
    }
}

fn records(output: &[u8]) -> Vec<Value> {
    std::str::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn all_operations_preview_paginate_delete_and_preserve_other_scopes() {
    let mut f = Fixture::new().await;
    let before = f.snapshot().await;
    let mut output = Vec::new();
    let summary = f.run(ExecutionMode::ReadOnly, &mut output).await.unwrap();
    assert!(summary.read_only && summary.supported_scope_exhausted);
    assert!(!summary.account_erasure_complete);
    // 24 owned rows plus 11 nullable attribution operations.
    assert_eq!(summary.items, 35);
    let mut keys = BTreeSet::new();
    let mut actions = BTreeSet::new();
    for record in records(&output)
        .into_iter()
        .filter(|r| r["record"] == "proposed_batch")
    {
        let batch = record["keys"].as_array().unwrap();
        assert!(!batch.is_empty() && batch.len() <= 2);
        actions.insert((record["table"].to_string(), record["action"].to_string()));
        for key in batch {
            assert!(keys.insert((
                record["table"].to_string(),
                record["action"].to_string(),
                key.to_string()
            )));
        }
    }
    assert_eq!(keys.len(), 35);
    assert_eq!(actions.len(), 14);
    assert_eq!(f.snapshot().await, before);
    let summary = f.run(ExecutionMode::Delete, &mut Vec::new()).await.unwrap();
    assert_eq!(summary.items, 35);
    assert!(
        !summary.read_only
            && summary.supported_scope_exhausted
            && !summary.account_erasure_complete
    );
    f.assert_cleaned(&before, &[f.options.community]).await;
    assert_eq!(
        f.run(ExecutionMode::Delete, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0
    );
    f.assert_cleaned(&before, &[f.options.community]).await;
    let first = f.options.community;
    f.options.community = f.other;
    assert_eq!(
        f.run(ExecutionMode::Delete, &mut Vec::new())
            .await
            .unwrap()
            .items,
        35
    );
    f.assert_cleaned(&before, &[first, f.other]).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cap_is_global_and_resumes_in_the_middle_of_an_operation() {
    let mut f = Fixture::new().await;
    let before = f.snapshot().await;
    f.options.max_items = 1;
    let preview = f
        .run(ExecutionMode::ReadOnly, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(preview.items, 1);
    assert!(!preview.supported_scope_exhausted);
    assert_eq!(f.snapshot().await, before);
    let partial = f.run(ExecutionMode::Delete, &mut Vec::new()).await.unwrap();
    assert_eq!(partial.items, 1);
    assert!(!partial.supported_scope_exhausted);
    let mut expected = before.clone();
    let target_tokens = expected["api_tokens"].as_array_mut().unwrap();
    let first = target_tokens
        .iter()
        .filter(|r| {
            r["community_id"] == f.options.community.to_string()
                && r["owner_pubkey"] == format!("\\x{TARGET}")
        })
        .min_by_key(|r| r["id"].as_str().unwrap())
        .unwrap()["id"]
        .clone();
    target_tokens
        .retain(|r| !(r["community_id"] == f.options.community.to_string() && r["id"] == first));
    assert_eq!(f.snapshot().await, expected);
    // Exact remaining cap is complete, not an off-by-one incomplete result.
    f.options.max_items = 34;
    let rest = f.run(ExecutionMode::Delete, &mut Vec::new()).await.unwrap();
    assert_eq!(rest.items, 34);
    assert!(rest.supported_scope_exhausted);
    f.assert_cleaned(&before, &[f.options.community]).await;
}
