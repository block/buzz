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
            agent_pubkeys: vec![self.agent.public_key().to_hex()],
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
        let event = EventBuilder::new(Kind::Custom(kind), content)
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
    assert!(f
        .store
        .run_profile_batches(&f.options(100), true, None, &mut Vec::new())
        .await
        .is_err());
    assert_eq!(before["events"], export(&f.pool).await["events"]);
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
