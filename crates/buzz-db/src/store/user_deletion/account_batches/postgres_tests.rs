use super::*;
use crate::Db;
use sqlx::PgPool;

const HUMAN: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
const OTHER: &str = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";

pub(super) async fn fixture() -> (PgPool, UserDeletionStore, AccountBatchOptions) {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let community = Uuid::new_v4();
    // Repeated keys in a second community catch selectors missing tenant scope.
    for id in [community, Uuid::new_v4()] {
        sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
            .bind(id)
            .bind(format!("{id}.example.org"))
            .execute(&pool)
            .await
            .unwrap();
        for key in [HUMAN, OTHER] {
            sqlx::query("INSERT INTO users(community_id,pubkey) VALUES($1,decode($2,'hex'))")
                .bind(id)
                .bind(key)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO relay_members(community_id,pubkey,role,added_by) VALUES($1,$2,'member',$3)").bind(id).bind(key).bind(HUMAN).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO api_tokens(community_id,id,owner_pubkey,token_hash,name,scopes,revoked_at,revoked_by) VALUES($1,$2,decode($3,'hex'),decode($3,'hex'),'private credential label','[]',now(),decode($4,'hex'))")
                .bind(id).bind(if key==HUMAN {Uuid::from_u128(1)} else {Uuid::from_u128(2)}).bind(key).bind(HUMAN).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO pubkey_allowlist(community_id,pubkey,added_by) VALUES($1,decode($2,'hex'),decode($3,'hex'))").bind(id).bind(key).bind(HUMAN).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO personal_read_accounts(community_id,actor) VALUES($1,decode($2,'hex'))").bind(id).bind(key).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO archived_identities(community_id,pubkey,consent_path,actor,replaced_by,request_event_id) VALUES($1,$2,'self',$2,$3,'archive-event')").bind(id).bind(key).bind(HUMAN).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO channels(community_id,id,name,created_by) VALUES($1,$2,'test',decode($3,'hex'))").bind(id).bind(Uuid::from_u128(10)).bind(OTHER).execute(&pool).await.unwrap();
        for key in [HUMAN, OTHER] {
            sqlx::query("INSERT INTO channel_members(community_id,channel_id,pubkey,invited_by,removed_by,removed_at) VALUES($1,$2,decode($3,'hex'),decode($4,'hex'),decode($4,'hex'),now())")
                .bind(id).bind(Uuid::from_u128(10)).bind(key).bind(HUMAN).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO personal_read_frontiers(community_id,actor,channel_id,root_id,through_timestamp) SELECT $1,decode($2,'hex'),$3,decode(lpad(to_hex(n),64,'0'),'hex'),now() FROM generate_series(1,250) n")
                .bind(id).bind(key).bind(Uuid::from_u128(10)).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO relay_invites(community_id,created_by,token_hash,expires_at) VALUES($1,$2,decode($2,'hex'),now())").bind(id).bind(HUMAN).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO join_policy_acceptances(community_id,pubkey,policy_version) SELECT $1,$2,lpad(n::text,64,'0') FROM generate_series(1,3)n").bind(id).bind(HUMAN).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO parameterized_event_watermarks(community_id,pubkey,kind,d_tag,created_at,event_id) VALUES($1,decode($2,'hex'),30078,'settings',now(),decode($2,'hex'))").bind(id).bind(HUMAN).execute(&pool).await.unwrap();
    }
    let store = Db::from_pool(pool.clone()).user_deletion_store();
    (
        pool,
        store,
        AccountBatchOptions {
            community,
            pubkey: HUMAN.into(),
            batch_size: 100,
            max_items: 100_000,
        },
    )
}

pub(super) async fn snapshot(pool: &PgPool) -> Value {
    let mut result = serde_json::Map::new();
    for table in [
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
        "entity_deletion_requests",
        "entity_deletion_stage_history",
        "entity_deletion_approvals",
    ] {
        let query = format!("SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)),'[]'::jsonb) FROM {table} t");
        let rows: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(query))
            .fetch_one(pool)
            .await
            .unwrap();
        result.insert(table.into(), rows);
    }
    Value::Object(result)
}

fn proposals(output: &[u8]) -> Vec<Value> {
    String::from_utf8_lossy(output)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|line| line["record"] == "proposed_batch")
        .collect()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn preview_delete_preserve_other_people_and_tenants_and_resume_in_bounded_batches() {
    let (pool, store, mut options) = fixture().await;
    let before = snapshot(&pool).await;
    let mut preview = Vec::new();
    let summary = store
        .preview_account_batches(&options, &mut preview)
        .await
        .unwrap();
    assert_eq!(snapshot(&pool).await, before);
    assert!(
        summary.read_only && summary.supported_scope_exhausted && !summary.account_erasure_complete
    );
    assert!(!String::from_utf8_lossy(&preview).contains("private credential label"));
    assert!(!String::from_utf8_lossy(&preview).contains("batch_committed"));
    let planned = proposals(&preview);
    let frontiers: Vec<_> = planned
        .iter()
        .filter(|p| p["table"] == "personal_read_frontiers")
        .collect();
    assert_eq!(
        frontiers
            .iter()
            .map(|p| p["keys"].as_array().unwrap().len())
            .collect::<Vec<_>>(),
        [100, 100, 50]
    );
    assert!(planned
        .iter()
        .all(|p| p["keys"].as_array().unwrap().len() <= 100));

    // An invocation limit leaves the parent and unprinted children intact.
    options.max_items = 108;
    let mut first = Vec::new();
    let partial = store
        .delete_account_batches(&options, &mut first)
        .await
        .unwrap();
    assert_eq!(partial.items, 108);
    assert!(!partial.supported_scope_exhausted);
    let remaining:i64 = sqlx::query_scalar("SELECT count(*) FROM personal_read_frontiers WHERE community_id=$1 AND actor=decode($2,'hex')").bind(options.community).bind(HUMAN).fetch_one(&pool).await.unwrap();
    assert_eq!(remaining, 150);
    let parent:bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM personal_read_accounts WHERE community_id=$1 AND actor=decode($2,'hex'))").bind(options.community).bind(HUMAN).fetch_one(&pool).await.unwrap();
    assert!(parent);
    options.max_items = 100_000;
    let completed = store
        .delete_account_batches(&options, &mut Vec::new())
        .await
        .unwrap();
    assert!(completed.supported_scope_exhausted && !completed.account_erasure_complete);
    assert_eq!(partial.items + completed.items, summary.items);
    assert_eq!(
        store
            .delete_account_batches(&options, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0
    );

    let after = snapshot(&pool).await;
    // Other community stays byte-for-byte identical, including its provenance.
    for (table, rows) in before.as_object().unwrap() {
        let foreign = |rows: &Value| {
            rows.as_array()
                .unwrap()
                .iter()
                .filter(|r| r["community_id"] != options.community.to_string())
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(foreign(rows), foreign(&after[table]), "{table}");
    }
    for table in [
        "users",
        "parameterized_event_watermarks",
        "entity_deletion_requests",
        "entity_deletion_stage_history",
        "entity_deletion_approvals",
    ] {
        assert_eq!(before[table], after[table], "{table}");
    }
    let other_token = after["api_tokens"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["community_id"] == options.community.to_string())
        .unwrap();
    assert!(other_token["revoked_by"].is_null());
    assert!(!other_token["revoked_at"].is_null());
    let other_member = after["channel_members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["community_id"] == options.community.to_string())
        .unwrap();
    assert!(other_member["invited_by"].is_null() && other_member["removed_by"].is_null());
    assert!(!other_member["removed_at"].is_null());
    // Archive visibility survives even though nullable replacement attribution clears.
    assert_eq!(after["archived_identities"].as_array().unwrap().len(), 4);
}

struct FailOutput {
    needle: &'static str,
}
impl Write for FailOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if String::from_utf8_lossy(bytes).contains(self.needle) {
            return Err(std::io::Error::other("injected output failure"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn output_failure_rolls_back_uncommitted_batch_and_retry_finds_committed_work() {
    let (pool, store, options) = fixture().await;
    let before = snapshot(&pool).await;
    assert!(store
        .delete_account_batches(
            &options,
            &mut FailOutput {
                needle: "proposed_batch"
            }
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("output failed"));
    assert_eq!(before, snapshot(&pool).await);
    // Failure printing the receipt is after commit: never imply this rolled back.
    assert!(store
        .delete_account_batches(
            &options,
            &mut FailOutput {
                needle: "batch_committed"
            }
        )
        .await
        .is_err());
    let after = snapshot(&pool).await;
    assert_eq!(after["api_tokens"].as_array().unwrap().len(), 3);
    let mut preview = Vec::new();
    store
        .preview_account_batches(&options, &mut preview)
        .await
        .unwrap();
    assert_ne!(proposals(&preview)[0]["table"], "api_tokens");
    assert!(
        store
            .delete_account_batches(&options, &mut Vec::new())
            .await
            .unwrap()
            .supported_scope_exhausted
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn owners_agents_and_inactive_communities_refuse_before_any_mutation() {
    let (pool, store, options) = fixture().await;
    sqlx::query("INSERT INTO community_serving_write_leases(community_id,operation,owner,fence_generation,lease_until) VALUES($1,'test','test-worker',0,now()+interval '1 hour')")
        .bind(options.community).execute(&pool).await.unwrap();
    let before = snapshot(&pool).await;
    assert!(store
        .delete_account_batches(&options, &mut Vec::new())
        .await
        .is_err());
    assert_eq!(before, snapshot(&pool).await);
    sqlx::query("DELETE FROM community_serving_write_leases WHERE community_id=$1")
        .bind(options.community)
        .execute(&pool)
        .await
        .unwrap();
    for (set, reset) in [
        (
            "UPDATE relay_members SET role='owner' WHERE community_id=$1 AND pubkey=$2",
            "UPDATE relay_members SET role='member' WHERE community_id=$1 AND pubkey=$2",
        ),
        (
            "UPDATE users SET agent_type='test' WHERE community_id=$1 AND pubkey=decode($2,'hex')",
            "UPDATE users SET agent_type=NULL WHERE community_id=$1 AND pubkey=decode($2,'hex')",
        ),
    ] {
        sqlx::query(set)
            .bind(options.community)
            .bind(HUMAN)
            .execute(&pool)
            .await
            .unwrap();
        let before = snapshot(&pool).await;
        assert!(store
            .delete_account_batches(&options, &mut Vec::new())
            .await
            .is_err());
        assert_eq!(before, snapshot(&pool).await);
        sqlx::query(reset)
            .bind(options.community)
            .bind(HUMAN)
            .execute(&pool)
            .await
            .unwrap();
    }
    let mut tx = pool.begin().await.unwrap();
    // Use the existing executor fence to construct the inactive fixture.
    sqlx::query("SELECT set_config('buzz.deletion_executor_community',$1,true), set_config('buzz.deletion_fence_generation','0',true)")
        .bind(options.community.to_string()).execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE communities SET deletion_state='quiescing' WHERE id=$1")
        .bind(options.community)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let before = snapshot(&pool).await;
    assert!(store
        .delete_account_batches(&options, &mut Vec::new())
        .await
        .is_err());
    assert_eq!(before, snapshot(&pool).await);
}
