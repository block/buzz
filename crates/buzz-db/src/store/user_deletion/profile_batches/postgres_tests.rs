use super::*;
use nostr::{Event, EventBuilder, Keys, Kind, Tag, Timestamp};
use sqlx::PgPool;
use std::collections::BTreeMap;

struct Fixture {
    pool: PgPool,
    store: UserDeletionStore,
    community: Uuid,
    elsewhere: Uuid,
    human: Keys,
    agent: Keys,
    other: Keys,
    relay: Keys,
}
impl Fixture {
    async fn new() -> Self {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let store = crate::Db::from_pool(pool.clone()).user_deletion_store();
        let f = Self {
            pool,
            store,
            community: Uuid::new_v4(),
            elsewhere: Uuid::new_v4(),
            human: Keys::generate(),
            agent: Keys::generate(),
            other: Keys::generate(),
            relay: Keys::generate(),
        };
        for c in [f.community, f.elsewhere] {
            sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
                .bind(c)
                .bind(format!("{c}.example.org"))
                .execute(&f.pool)
                .await
                .unwrap();
            for k in [&f.human, &f.agent, &f.other] {
                sqlx::query("INSERT INTO users(community_id,pubkey,display_name) VALUES($1,$2,'profile fixture')").bind(c).bind(k.public_key().to_bytes().as_slice()).execute(&f.pool).await.unwrap();
            }
            sqlx::query(
                "UPDATE users SET agent_owner_pubkey=$2 WHERE community_id=$1 AND pubkey=$3",
            )
            .bind(c)
            .bind(f.human.public_key().to_bytes().as_slice())
            .bind(f.agent.public_key().to_bytes().as_slice())
            .execute(&f.pool)
            .await
            .unwrap();
        }
        f
    }
    fn options(&self, batch_size: u16) -> ProfileBatchOptions {
        ProfileBatchOptions {
            community: self.community,
            pubkey: self.human.public_key().to_hex(),
            relay_pubkeys: vec![self.relay.public_key().to_hex()],
            batch_size,
            max_items: 1000,
        }
    }
    async fn event(
        &self,
        c: Uuid,
        key: &Keys,
        kind: u16,
        tags: Vec<Vec<String>>,
        content: &str,
    ) -> Event {
        // Self-archive requests must retain the author as their explicit p subject.
        let event = EventBuilder::new(Kind::Custom(kind), content)
            .allow_self_tagging()
            .tags(tags.into_iter().map(|v| Tag::parse(v).unwrap()))
            .custom_created_at(Timestamp::now())
            .sign_with_keys(key)
            .unwrap();
        sqlx::query("INSERT INTO events(community_id,id,pubkey,created_at,kind,tags,content,sig) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(c).bind(event.id.to_bytes().as_slice()).bind(event.pubkey.to_bytes().as_slice())
            .bind(chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64,0).unwrap())
            .bind(i32::from(kind)).bind(serde_json::to_value(&event.tags).unwrap()).bind(&event.content).bind(event.sig.serialize().as_slice())
            .execute(&self.pool).await.unwrap();
        event
    }
    async fn profiles(&self) {
        for c in [self.community, self.elsewhere] {
            self.event(c, &self.human, 0, vec![], "human profile").await;
            self.event(c, &self.agent, 10100, vec![], "agent profile")
                .await;
            self.event(c, &self.other, 0, vec![], "surviving profile")
                .await;
            self.event(
                c,
                &self.human,
                30177,
                vec![vec!["d".into(), self.agent.public_key().to_hex()]],
                "configuration",
            )
            .await;
        }
        let subject = self.human.public_key().to_hex();
        let event = self
            .event(
                self.community,
                &self.relay,
                30382,
                vec![
                    vec!["d".into(), subject.clone()],
                    vec!["p".into(), subject.clone()],
                ],
                "",
            )
            .await;
        sqlx::query("INSERT INTO nip_fi_profile_heads(community_id,pubkey,issuer,issued_at,event) VALUES($1,$2,'https://issuer.example.org',1,$3)")
            .bind(self.community).bind(&subject).bind(serde_json::to_value(&event).unwrap()).execute(&self.pool).await.unwrap();
        sqlx::query("INSERT INTO event_mentions(community_id,pubkey_hex,event_id,event_created_at,event_kind) VALUES($1,$2,$3,now(),30382)")
            .bind(self.community).bind(&subject).bind(event.id.to_bytes().as_slice()).execute(&self.pool).await.unwrap();
        for key in [&self.human, &self.other] {
            sqlx::query("INSERT INTO archived_identities(community_id,pubkey,consent_path,actor,request_event_id) VALUES($1,$2,'self',$2,$3)")
                .bind(self.community).bind(key.public_key().to_hex()).bind("a".repeat(64)).execute(&self.pool).await.unwrap();
        }
        self.event(
            self.community,
            &self.relay,
            13535,
            vec![
                vec!["-".into()],
                vec!["p".into(), subject],
                vec!["p".into(), self.other.public_key().to_hex()],
            ],
            "",
        )
        .await;
    }
}

async fn export(pool: &PgPool) -> BTreeMap<String, Value> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT tablename FROM pg_tables WHERE schemaname='public' ORDER BY tablename",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let mut result = BTreeMap::new();
    for table in tables {
        let sql=format!("SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM public.\"{}\" t",table.replace('"',"\"\""));
        result.insert(
            table,
            sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
                .fetch_one(pool)
                .await
                .unwrap(),
        );
    }
    result
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn preview_delete_retry_preserve_other_identities_and_communities() {
    let f = Fixture::new().await;
    f.profiles().await;
    let before = export(&f.pool).await;
    let mut preview = Vec::new();
    let opts = f.options(2);
    let expected = f
        .store
        .run_profile_batches(&opts, false, None, &mut preview)
        .await
        .unwrap();
    assert_eq!(before, export(&f.pool).await, "preview changed database");
    assert!(expected.items > 2);
    assert!(expected.batches > 1);
    let output = String::from_utf8(preview).unwrap();
    assert!(output.contains("replace_archive_snapshot"));
    assert!(!output.contains("configuration"));
    assert!(!output.contains("human profile"));
    let mut deletion = Vec::new();
    let actual = f
        .store
        .run_profile_batches(
            &opts,
            true,
            Some(&f.relay.secret_key().to_secret_hex()),
            &mut deletion,
        )
        .await
        .unwrap();
    assert_eq!(actual.items, expected.items);
    let after = export(&f.pool).await;
    for (table, rows) in &before {
        if ![
            "events",
            "users",
            "nip_fi_profile_heads",
            "archived_identities",
            "event_mentions",
        ]
        .contains(&table.as_str())
            && !table.starts_with("events_")
        {
            assert_eq!(rows, &after[table], "unexpected change to {table}");
        }
    }
    let remaining: Vec<Vec<u8>> =
        sqlx::query_scalar("SELECT pubkey FROM users WHERE community_id=$1")
            .bind(f.community)
            .fetch_all(&f.pool)
            .await
            .unwrap();
    assert_eq!(remaining, vec![f.other.public_key().to_bytes().to_vec()]);
    for (table, old_rows) in &before {
        for row in old_rows.as_array().unwrap() {
            let key = row["pubkey"]
                .as_str()
                .unwrap_or("")
                .trim_start_matches("\\x");
            if row["community_id"] == json!(f.elsewhere) || key == f.other.public_key().to_hex() {
                assert!(
                    after[table].as_array().unwrap().contains(row),
                    "survivor changed in {table}"
                );
            }
        }
    }
    let other_count: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE community_id=$1")
        .bind(f.elsewhere)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(other_count, 3);
    let snapshot=sqlx::query("SELECT id,pubkey,extract(epoch FROM created_at)::bigint AS created_at,kind,tags,content,sig FROM events WHERE community_id=$1 AND kind=13535").bind(f.community).fetch_one(&f.pool).await.unwrap();
    use sqlx::Row;
    let signed:Event=serde_json::from_value(json!({"id":hex::encode(snapshot.get::<Vec<u8>,_>("id")),"pubkey":hex::encode(snapshot.get::<Vec<u8>,_>("pubkey")),"created_at":snapshot.get::<i64,_>("created_at"),"kind":13535,"tags":snapshot.get::<Value,_>("tags"),"content":snapshot.get::<String,_>("content"),"sig":hex::encode(snapshot.get::<Vec<u8>,_>("sig"))})).unwrap();
    signed.verify().unwrap();
    assert_eq!(
        signed
            .tags
            .iter()
            .filter(|t| t.as_slice().first().map(String::as_str) == Some("p"))
            .count(),
        1
    );
    assert!(serde_json::to_string(&signed.tags)
        .unwrap()
        .contains(&f.other.public_key().to_hex()));
    assert!(!serde_json::to_string(&signed.tags)
        .unwrap()
        .contains(&f.human.public_key().to_hex()));
    let second = f
        .store
        .run_profile_batches(&opts, true, None, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(second.items, 0);
    assert_eq!(after, export(&f.pool).await);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn limits_dependencies_and_signer_fail_before_any_mutation() {
    let f = Fixture::new().await;
    f.profiles().await;
    let before = export(&f.pool).await;
    let mut opts = f.options(1);
    opts.max_items = 1;
    assert!(f
        .store
        .run_profile_batches(&opts, true, None, &mut Vec::new())
        .await
        .is_err());
    opts.max_items = 1000;
    for key in [None, Some(f.other.secret_key().to_secret_hex())] {
        assert!(f
            .store
            .run_profile_batches(&opts, true, key.as_deref(), &mut Vec::new())
            .await
            .is_err());
        assert_eq!(before, export(&f.pool).await);
    }
    opts.relay_pubkeys.clear();
    assert!(f
        .store
        .run_profile_batches(&opts, false, None, &mut Vec::new())
        .await
        .is_err());
    assert_eq!(before, export(&f.pool).await);
    opts = f.options(1);
    f.event(
        f.community,
        &f.agent,
        1,
        vec![],
        "content must be cleaned first",
    )
    .await;
    let before = export(&f.pool).await;
    let mut output = Vec::new();
    assert!(f
        .store
        .run_profile_batches(
            &opts,
            true,
            Some(&f.relay.secret_key().to_secret_hex()),
            &mut output
        )
        .await
        .is_err());
    assert!(String::from_utf8(output)
        .unwrap()
        .contains("content/account cleanup"));
    assert_eq!(before, export(&f.pool).await);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn content_then_profile_cleanup_retains_human_and_agent_repositories() {
    use crate::user_deletion::{content_batches::ContentBatchOptions, ExecutionMode};

    let f = Fixture::new().await;
    f.profiles().await;
    let mut repository_ids = Vec::new();
    for community in [f.community, f.elsewhere] {
        for owner in [&f.human, &f.agent, &f.other] {
            let repo = format!("repo-{}", owner.public_key());
            for kind in [30617, 30618] {
                let event = f
                    .event(
                        community,
                        owner,
                        kind,
                        vec![vec!["d".into(), repo.clone()]],
                        "retained repository metadata",
                    )
                    .await;
                repository_ids.push(json!(format!("\\x{}", event.id)));
                sqlx::query("INSERT INTO event_mentions(community_id,pubkey_hex,event_id,event_created_at,event_kind) VALUES($1,$2,$3,now(),$4)")
                    .bind(community).bind(f.other.public_key().to_hex())
                    .bind(event.id.to_bytes().as_slice()).bind(i32::from(kind))
                    .execute(&f.pool).await.unwrap();
            }
            sqlx::query(
                "INSERT INTO git_repo_names(community_id,repo_id,owner_pubkey) VALUES($1,$2,$3)",
            )
            .bind(community)
            .bind(repo)
            .bind(owner.public_key().to_hex())
            .execute(&f.pool)
            .await
            .unwrap();
        }
    }
    // Retaining repositories must not exempt ordinary human or agent content.
    for owner in [&f.human, &f.agent] {
        f.event(f.community, owner, 1, vec![], "remove content first")
            .await;
    }
    let before = export(&f.pool).await;
    let opts = f.options(2);
    let secret = f.relay.secret_key().to_secret_hex();
    let mut blocked = Vec::new();
    assert!(f
        .store
        .run_profile_batches(&opts, true, Some(&secret), &mut blocked)
        .await
        .is_err());
    assert!(String::from_utf8(blocked)
        .unwrap()
        .contains("events: content/account cleanup"));
    assert_eq!(before, export(&f.pool).await);
    f.store
        .run_message_batches(
            &ContentBatchOptions {
                community: f.community,
                pubkey: f.human.public_key().to_hex(),
                relay_pubkey: Some(f.relay.public_key().to_hex()),
                batch_size: 2,
                max_items: 1000,
            },
            ExecutionMode::Delete,
            None,
            &mut Vec::new(),
        )
        .await
        .unwrap();
    let after_content = export(&f.pool).await;
    let mut preview = Vec::new();
    let proposed = f
        .store
        .run_profile_batches(&opts, false, None, &mut preview)
        .await
        .unwrap();
    let start: Value =
        serde_json::from_slice(preview.split(|b| *b == b'\n').next().unwrap()).unwrap();
    assert_eq!(start["blockers"], json!([]));
    assert_eq!(after_content, export(&f.pool).await);
    let deleted = f
        .store
        .run_profile_batches(&opts, true, Some(&secret), &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(proposed.items, deleted.items);
    let after = export(&f.pool).await;
    assert_eq!(before["git_repo_names"], after["git_repo_names"]);
    for (table, rows) in &before {
        for row in rows.as_array().unwrap() {
            if row["community_id"] == json!(f.elsewhere)
                || (table == "events" && repository_ids.contains(&row["id"]))
                || (table == "event_mentions" && repository_ids.contains(&row["event_id"]))
            {
                assert!(
                    after[table].as_array().unwrap().contains(row),
                    "retained row changed in {table}"
                );
            }
        }
    }
    let remaining: Vec<Vec<u8>> =
        sqlx::query_scalar("SELECT pubkey FROM users WHERE community_id=$1")
            .bind(f.community)
            .fetch_all(&f.pool)
            .await
            .unwrap();
    assert_eq!(remaining, vec![f.other.public_key().to_bytes().to_vec()]);
    assert_eq!(
        f.store
            .run_profile_batches(&opts, true, None, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0
    );
    assert_eq!(after, export(&f.pool).await);
}

struct FailAfterCommit;
impl Write for FailAfterCommit {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if String::from_utf8_lossy(bytes).contains("batch_committed") {
            Err(std::io::Error::other("injected output loss"))
        } else {
            Ok(bytes.len())
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[tokio::test]
#[ignore = "requires Postgres"]
async fn commit_before_output_loss_resumes_from_same_target_scope() {
    let f = Fixture::new().await;
    f.profiles().await;
    let opts = f.options(2);
    assert!(f
        .store
        .run_profile_batches(
            &opts,
            true,
            Some(&f.relay.secret_key().to_secret_hex()),
            &mut FailAfterCommit
        )
        .await
        .is_err());
    let mut output = Vec::new();
    f.store
        .run_profile_batches(
            &opts,
            true,
            Some(&f.relay.secret_key().to_secret_hex()),
            &mut output,
        )
        .await
        .unwrap();
    assert!(String::from_utf8(output)
        .unwrap()
        .contains("delete_complete"));
    assert_eq!(
        f.store
            .run_profile_batches(&opts, true, None, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn changed_source_rolls_back_batch_and_changed_owner_is_refused() {
    let f = Fixture::new().await;
    f.event(f.community, &f.human, 0, vec![], "initial").await;
    let mut conn = f.pool.acquire().await.unwrap();
    let plan = plan::build(&mut conn, &f.options(100)).await.unwrap();
    sqlx::query("UPDATE events SET content='changed' WHERE community_id=$1")
        .bind(f.community)
        .execute(&f.pool)
        .await
        .unwrap();
    let mut raw = f.pool.begin().await.unwrap();
    lock(&mut raw, &plan.scope).await.unwrap();
    let mut tx = crate::AdmittedTx::admit(
        raw,
        &crate::deletion::DeletionStore::new(f.pool.clone()),
        buzz_core::CommunityId::from_uuid(f.community),
    )
    .await
    .unwrap();
    assert!(
        rows::apply(&mut tx, &plan.scope, &plan.ops[0], &Default::default())
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    let before = export(&f.pool).await;
    sqlx::query("UPDATE users SET agent_owner_pubkey=$2 WHERE community_id=$1 AND pubkey=$3")
        .bind(f.community)
        .bind(f.other.public_key().to_bytes().as_slice())
        .bind(f.agent.public_key().to_bytes().as_slice())
        .execute(&f.pool)
        .await
        .unwrap();
    let mut tx = f.pool.begin().await.unwrap();
    assert!(
        ownership::recheck(&mut tx, &plan.scope, &plan.scope.targets)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    assert_eq!(before["events"], export(&f.pool).await["events"]);
    // A new invocation excludes the transferred agent instead of trusting the old plan.
    let result = f
        .store
        .run_profile_batches(&f.options(100), true, None, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(result.target_pubkeys, vec![f.human.public_key().to_hex()]);
    let owner: Vec<u8> = sqlx::query_scalar(
        "SELECT agent_owner_pubkey FROM users WHERE community_id=$1 AND pubkey=$2",
    )
    .bind(f.community)
    .bind(f.agent.public_key().to_bytes().as_slice())
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(owner, f.other.public_key().to_bytes());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn mismatched_verified_subject_and_survivor_archive_actor_are_not_erased() {
    let f = Fixture::new().await;
    f.event(
        f.community,
        &f.relay,
        30382,
        vec![
            vec!["p".into(), f.human.public_key().to_hex()],
            vec!["d".into(), f.other.public_key().to_hex()],
        ],
        "",
    )
    .await;
    let before = export(&f.pool).await;
    assert!(f
        .store
        .run_profile_batches(&f.options(100), true, None, &mut Vec::new())
        .await
        .is_err());
    assert_eq!(before, export(&f.pool).await);
}

// Hooks run at the public handler's output boundary, exercising the real batch path.
struct RecordHook<F>(F);
impl<F: FnMut(Value) -> std::io::Result<()>> Write for RecordHook<F> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if let Ok(record) = serde_json::from_slice(bytes) {
            (self.0)(record)?;
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn community_ownership_and_retry_after_identity_removal() {
    // Ownership elsewhere is neither conflicting local ownership nor missing local evidence.
    for missing in [false, true] {
        let f = Fixture::new().await;
        f.event(f.community, &f.human, 0, vec![], "human").await;
        f.event(f.community, &f.agent, 10100, vec![], "retain agent")
            .await;
        if missing {
            sqlx::query("DELETE FROM users WHERE community_id=$1 AND pubkey=$2")
                .bind(f.community)
                .bind(f.agent.public_key().to_bytes().as_slice())
                .execute(&f.pool)
                .await
                .unwrap();
        } else {
            sqlx::query(
                "UPDATE users SET agent_owner_pubkey=$3 WHERE community_id=$1 AND pubkey=$2",
            )
            .bind(f.community)
            .bind(f.agent.public_key().to_bytes().as_slice())
            .bind(f.other.public_key().to_bytes().as_slice())
            .execute(&f.pool)
            .await
            .unwrap();
        }
        let before = export(&f.pool).await;
        let preview = f
            .store
            .run_profile_batches(&f.options(1), false, None, &mut Vec::new())
            .await
            .unwrap();
        assert_eq!(preview.target_pubkeys, vec![f.human.public_key().to_hex()]);
        assert_eq!(before, export(&f.pool).await);
        f.store
            .run_profile_batches(&f.options(1), true, None, &mut Vec::new())
            .await
            .unwrap();
        let after = export(&f.pool).await;
        for (table, rows) in &before {
            for row in rows.as_array().unwrap() {
                if row["community_id"] == json!(f.elsewhere)
                    || row["pubkey"] == format!("\\x{}", f.agent.public_key())
                {
                    assert!(
                        after[table].as_array().unwrap().contains(row),
                        "survivor changed in {table}"
                    );
                }
            }
        }
    }
    let f = Fixture::new().await;
    f.event(f.community, &f.human, 0, vec![], "human").await;
    f.event(f.community, &f.agent, 10100, vec![], "agent").await;
    let mut removing_agent = false;
    let mut output = RecordHook(|record: Value| {
        if record["record"] == "proposed_batch" {
            removing_agent = record["items"][0]["table"] == "users"
                && record["items"][0]["key"]["pubkey"] == format!("\\x{}", f.agent.public_key());
        }
        if record["record"] == "batch_committed" && removing_agent {
            return Err(std::io::Error::other(
                "lost output after agent identity commit",
            ));
        }
        Ok(())
    });
    assert!(f
        .store
        .run_profile_batches(&f.options(1), true, None, &mut output)
        .await
        .is_err());
    let remaining: Vec<Vec<u8>> =
        sqlx::query_scalar("SELECT pubkey FROM users WHERE community_id=$1")
            .bind(f.community)
            .fetch_all(&f.pool)
            .await
            .unwrap();
    assert!(!remaining.contains(&f.agent.public_key().to_bytes().to_vec()));
    assert!(remaining.contains(&f.human.public_key().to_bytes().to_vec()));
    let agent_events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND pubkey=$2")
            .bind(f.community)
            .bind(f.agent.public_key().to_bytes().as_slice())
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(
        agent_events, 0,
        "identity removal stranded undiscoverable profile data"
    );

    let retry = f
        .store
        .run_profile_batches(&f.options(1), true, None, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(retry.target_pubkeys, vec![f.human.public_key().to_hex()]);
    assert_eq!(retry.items, 1);
    assert_eq!(
        f.store
            .run_profile_batches(&f.options(1), true, None, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Postgres"]
async fn ownership_changes_before_write_are_refused_and_retry_rediscovers() {
    for change in ["transfer", "missing", "new"] {
        let f = Fixture::new().await;
        f.event(f.community, &f.human, 0, vec![], "human").await;
        f.event(f.community, &f.agent, 10100, vec![], "agent").await;
        let mut changed = false;
        let mut output = RecordHook(|record: Value| {
            if record["record"] == "proposed_batch" && !changed {
                changed = true;
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                    let sql = match change {
                        "transfer" => "UPDATE users SET agent_owner_pubkey=$3 WHERE community_id=$1 AND pubkey=$2",
                        "missing" => "UPDATE users SET agent_owner_pubkey=NULL WHERE community_id=$1 AND pubkey=$2 AND $3::bytea IS NOT NULL",
                        _ => "UPDATE users SET agent_owner_pubkey=$2 WHERE community_id=$1 AND pubkey=$3",
                    };
                    let key = if change == "new" { &f.human } else { &f.agent };
                    sqlx::query(sql).bind(f.community).bind(key.public_key().to_bytes().as_slice())
                        .bind(f.other.public_key().to_bytes().as_slice()).execute(&f.pool).await.unwrap();
                })
                });
            }
            Ok(())
        });
        let before = export(&f.pool).await;
        let error = f
            .store
            .run_profile_batches(&f.options(1), true, None, &mut output)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("ownership changed"), "{error}");
        assert!(changed);
        assert_eq!(before["events"], export(&f.pool).await["events"]);
        let retry = f
            .store
            .run_profile_batches(&f.options(1), true, None, &mut Vec::new())
            .await
            .unwrap();
        assert_eq!(
            retry.target_pubkeys.len(),
            if change == "new" { 3 } else { 1 }
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Postgres"]
async fn final_check_keeps_removed_agent_scope_and_locks_ownership() {
    let f = Fixture::new().await;
    f.event(f.community, &f.agent, 10100, vec![], "initial")
        .await;
    let mut conn = f.pool.acquire().await.unwrap();
    let plan = plan::build(&mut conn, &f.options(1)).await.unwrap();
    let mut checked = f.pool.begin().await.unwrap();
    ownership::recheck(&mut checked, &plan.scope, &plan.scope.targets)
        .await
        .unwrap();
    let mut transfer = f.pool.begin().await.unwrap();
    sqlx::query("SET LOCAL lock_timeout='100ms'")
        .execute(&mut *transfer)
        .await
        .unwrap();
    let error =
        sqlx::query("UPDATE users SET agent_owner_pubkey=$3 WHERE community_id=$1 AND pubkey=$2")
            .bind(f.community)
            .bind(f.agent.public_key().to_bytes().as_slice())
            .bind(f.other.public_key().to_bytes().as_slice())
            .execute(&mut *transfer)
            .await
            .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("55P03")
    );
    transfer.rollback().await.unwrap();
    checked.rollback().await.unwrap();
    drop(conn);

    let mut removing_human = false;
    let mut inserted = false;
    let mut output = RecordHook(|record: Value| {
        if record["record"] == "proposed_batch" {
            removing_human = record["items"][0]["table"] == "users"
                && record["items"][0]["key"]["pubkey"] == format!("\\x{}", f.human.public_key());
        }
        if record["record"] == "batch_committed" && removing_human {
            // Simulate an out-of-contract writer: final verification must still see
            // the erased agent's residual data after ownership discovery shrinks.
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(f.event(
                    f.community,
                    &f.agent,
                    10100,
                    vec![],
                    "late residual profile",
                ))
            });
            inserted = true;
        }
        Ok(())
    });
    let error = f
        .store
        .run_profile_batches(&f.options(1), true, None, &mut output)
        .await
        .unwrap_err();
    assert!(inserted);
    assert!(
        error
            .to_string()
            .contains("profile rows or dependencies remain"),
        "{error}"
    );
}

async fn historical_replacement_blocks_cleanup_after_unarchive(kind: u16) {
    for agent in [false, true] {
        let f = Fixture::new().await;
        let replacement = if agent { &f.agent } else { &f.human };
        let subject = f.other.public_key().to_hex();
        let replacement = replacement.public_key().to_hex();
        let signer = if kind == 9035 { &f.other } else { &f.relay };
        let mut tags = vec![
            vec!["-".into()],
            vec!["p".into(), subject.clone()],
            vec!["replaced-by".into(), replacement.clone()],
        ];
        if kind == 8002 {
            tags.push(vec!["consent".into(), "relay".into()]);
        }
        let event = f.event(f.community, signer, kind, tags, "").await;
        event.verify().unwrap();
        let community = buzz_core::CommunityId::from_uuid(f.community);
        assert!(crate::archived_identities::archive(
            &f.pool,
            community,
            &subject,
            "self",
            &subject,
            None,
            Some(&replacement),
            &event.id.to_hex(),
        )
        .await
        .unwrap());
        assert!(
            crate::archived_identities::unarchive(&f.pool, community, &subject)
                .await
                .unwrap()
        );
        assert!(
            !crate::archived_identities::is_archived(&f.pool, community, &subject)
                .await
                .unwrap()
        );
        let before = export(&f.pool).await;
        for delete in [false, true] {
            let mut output = Vec::new();
            let result = f
                .store
                .run_profile_batches(&f.options(1), delete, None, &mut output)
                .await;
            assert!(result.is_err(), "kind {kind}, agent {agent}, delete {delete}: unresolved historical replacement was accepted");
            let error = result.unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("shared-history disposition required"),
                "{error}"
            );
            assert!(!String::from_utf8(output)
                .unwrap()
                .contains("delete_complete"));
            assert_eq!(
                before,
                export(&f.pool).await,
                "blocked cleanup mutated database"
            );
        }
        // The same historical reference in another community does not block cleanup here.
        sqlx::query("UPDATE events SET community_id=$2 WHERE community_id=$1 AND id=$3")
            .bind(f.community)
            .bind(f.elsewhere)
            .bind(event.id.to_bytes().as_slice())
            .execute(&f.pool)
            .await
            .unwrap();
        let foreign = export(&f.pool).await;
        f.store
            .run_profile_batches(&f.options(1), true, None, &mut Vec::new())
            .await
            .unwrap();
        assert_eq!(foreign["events"], export(&f.pool).await["events"]);
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn historical_archive_request_replacement_blocks_after_unarchive() {
    historical_replacement_blocks_cleanup_after_unarchive(9035).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn historical_archive_delta_replacement_blocks_after_unarchive() {
    historical_replacement_blocks_cleanup_after_unarchive(8002).await;
}
