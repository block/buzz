use super::{failures_postgres_tests::RecordWriter, *};

const AGENT: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const OTHER_OWNER: &str = "3333333333333333333333333333333333333333333333333333333333333333";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Postgres"]
async fn ownership_changes_stop_batches_and_retry_fetches_current_agents() {
    let mut f = Fixture::new().await;
    // Missing ownership, transferred ownership, and newly assigned ownership
    // all invalidate a running handler's set. No old key list is sufficient.
    for (initial, next, after_table) in [
        (Some(TARGET), None, "api_tokens"),
        (Some(TARGET), Some(OTHER_OWNER), "relay_invites"),
        (None, Some(TARGET), "api_tokens"),
    ] {
        sqlx::query("UPDATE users SET agent_owner_pubkey=decode($1,'hex') WHERE community_id=$2 AND pubkey=decode($3,'hex')")
            .bind(initial).bind(f.options.community).bind(AGENT).execute(&f.pool).await.unwrap();
        // Preview invokes discovery too and leaves every table untouched.
        let before = f.snapshot().await;
        let mut preview = Vec::new();
        f.run(ExecutionMode::ReadOnly, &mut preview).await.unwrap();
        assert_eq!(f.snapshot().await, before);
        let targets = records(&preview)
            .into_iter()
            .find(|r| r["record"] == "account_targets")
            .unwrap();
        assert_eq!(targets["agent_count"], usize::from(initial == Some(TARGET)));

        // Refill the human's first operations so the change happens between
        // real committed batches, including immediately before memberships.
        sqlx::query("INSERT INTO api_tokens(community_id,id,owner_pubkey,token_hash,name,scopes) VALUES ($1,gen_random_uuid(),decode($2,'hex'),decode(repeat(md5(random()::text),2),'hex'),'ownership test','[]')")
            .bind(f.options.community).bind(TARGET).execute(&f.pool).await.unwrap();
        let mut changed = false;
        let mut writer = RecordWriter::new(|r: &Value| {
            if !changed && r["record"] == "batch_committed" && r["table"] == after_table {
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                    sqlx::query("UPDATE users SET agent_owner_pubkey=decode($1,'hex') WHERE community_id=$2 AND pubkey=decode($3,'hex')")
                        .bind(next).bind(f.options.community).bind(AGENT).execute(&f.pool).await.unwrap();
                })
                });
                changed = true;
            }
            Ok(())
        });
        let error = f.run(ExecutionMode::Delete, &mut writer).await.unwrap_err();
        assert!(changed);
        assert!(error.to_string().contains("agent ownership changed"));
        let remaining: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM api_tokens WHERE owner_pubkey=decode($1,'hex')",
        )
        .bind(AGENT)
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert_eq!(remaining, 6, "agent rows survive the invalidated run");
    }
    // Retry picks up the addition, cleans only this community, and leaves the
    // ownership source available for later handlers and further retries.
    f.run(ExecutionMode::Delete, &mut Vec::new()).await.unwrap();
    let remaining: Vec<Uuid> = sqlx::query_scalar(
        "SELECT community_id FROM api_tokens WHERE owner_pubkey=decode($1,'hex')",
    )
    .bind(AGENT)
    .fetch_all(&f.pool)
    .await
    .unwrap();
    assert_eq!(remaining, vec![f.other; 3]);
    let owner: String = sqlx::query_scalar("SELECT encode(agent_owner_pubkey,'hex') FROM users WHERE community_id=$1 AND pubkey=decode($2,'hex')")
        .bind(f.options.community).bind(AGENT).fetch_one(&f.pool).await.unwrap();
    assert_eq!(owner, TARGET);
    assert_eq!(
        f.run(ExecutionMode::Delete, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0
    );
    // Large valid discovery sets must not exceed a single JSON record's bound.
    // A small row budget still permits preview and deletion with 4,001 agents.
    sqlx::query("INSERT INTO users(community_id,pubkey,agent_owner_pubkey) SELECT $1,decode(lpad(to_hex(n),64,'0'),'hex'),decode($2,'hex') FROM generate_series(1,4000) n")
        .bind(f.options.community).bind(TARGET).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO api_tokens(community_id,id,owner_pubkey,token_hash,name,scopes) SELECT $1,gen_random_uuid(),decode($2,'hex'),decode(repeat(md5(random()::text),2),'hex'),'large discovery','[]' FROM generate_series(1,2)")
        .bind(f.options.community).bind(TARGET).execute(&f.pool).await.unwrap();
    f.options.max_items = 1;
    for mode in [ExecutionMode::ReadOnly, ExecutionMode::Delete] {
        let mut output = Vec::new();
        let summary = f.run(mode, &mut output).await.unwrap();
        assert_eq!(summary.items, 1);
        assert!(!summary.supported_scope_exhausted);
        assert_eq!(records(&output)[0]["agent_count"], 4001);
    }
}
