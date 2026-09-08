use super::*;
use buzz_core::kind::KIND_INTERACTION_STATE;
use nostr::Timestamp;

struct Fixture {
    db: Db,
    community: CommunityId,
    channel: Uuid,
    asker: Keys,
    alice: Keys,
    bob: Keys,
    relay: Keys,
}
impl Fixture {
    async fn new() -> Self {
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let db = Db::from_pool(pool.clone());
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let channel = Uuid::new_v4();
        let asker = Keys::generate();
        let alice = Keys::generate();
        let bob = Keys::generate();
        sqlx::query("INSERT INTO communities(id,host) VALUES($1,$2)")
            .bind(community.as_uuid())
            .bind(format!("interactions-{}.test", Uuid::new_v4()))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO channels(id,community_id,name,created_by) VALUES($1,$2,'interactions',$3)",
        )
        .bind(channel)
        .bind(community.as_uuid())
        .bind(asker.public_key().to_bytes().as_slice())
        .execute(&pool)
        .await
        .unwrap();
        for key in [&asker, &alice, &bob] {
            sqlx::query(
                "INSERT INTO channel_members(community_id,channel_id,pubkey) VALUES($1,$2,$3)",
            )
            .bind(community.as_uuid())
            .bind(channel)
            .bind(key.public_key().to_bytes().as_slice())
            .execute(&pool)
            .await
            .unwrap();
        }
        Self {
            db,
            community,
            channel,
            asker,
            alice,
            bob,
            relay: Keys::generate(),
        }
    }
    fn prompt(&self, closes: &str, responders: &str) -> Event {
        self.event(
            &self.asker,
            KIND_INTERACTION_PROMPT,
            "Choose",
            vec![
                vec!["itype", "buttons"],
                vec!["opt", "yes", "Yes"],
                vec!["opt", "no", "No"],
                vec!["closes", closes],
                vec!["responders", responders],
                vec!["deadline", &(Timestamp::now().as_secs() + 3600).to_string()],
            ],
        )
    }
    fn event(&self, keys: &Keys, kind: u32, content: &str, ts: Vec<Vec<&str>>) -> Event {
        let mut tags = vec![Tag::parse(["h", &self.channel.to_string()]).unwrap()];
        tags.extend(ts.into_iter().map(|t| Tag::parse(t).unwrap()));
        EventBuilder::new(Kind::Custom(kind as u16), content)
            .tags(tags)
            .sign_with_keys(keys)
            .unwrap()
    }
    fn answer(&self, key: &Keys, p: &Event, choice: &str) -> Event {
        self.event(
            key,
            KIND_INTERACTION_RESPONSE,
            "",
            vec![
                vec!["e", &p.id.to_hex(), "", "prompt"],
                vec!["choice", choice],
            ],
        )
    }
    async fn accept(&self, e: &Event) -> Result<Option<Vec<StoredEvent>>> {
        self.db
            .accept_interaction(self.community, e, &self.relay, None)
            .await
    }
    async fn state(&self, p: &Event) -> InteractionState {
        let value: serde_json::Value = sqlx::query_scalar(
            "SELECT state FROM interactions WHERE community_id=$1 AND prompt_id=$2",
        )
        .bind(self.community.as_uuid())
        .bind(p.id.as_bytes().as_slice())
        .fetch_one(&self.db.pool)
        .await
        .unwrap();
        serde_json::from_value(value).unwrap()
    }
    async fn stored(&self, e: &Event) -> bool {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND id=$2)")
            .bind(self.community.as_uuid())
            .bind(e.id.as_bytes().as_slice())
            .fetch_one(&self.db.pool)
            .await
            .unwrap()
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn concurrent_first_answers_commit_exactly_one_decision() {
    let f = Fixture::new().await;
    let p = f.prompt("first", "members");
    let emitted = f.accept(&p).await.unwrap().unwrap();
    assert_eq!(emitted.len(), 3);
    assert_eq!(
        emitted
            .iter()
            .filter(|e| e.event.kind.as_u16() as u32 == KIND_STREAM_MESSAGE)
            .count(),
        1
    );
    let a = f.answer(&f.alice, &p, "yes");
    let b = f.answer(&f.bob, &p, "no");
    let (a_result, b_result) = tokio::join!(f.accept(&a), f.accept(&b));
    assert_ne!(a_result.is_ok(), b_result.is_ok());
    assert_ne!(f.stored(&a).await, f.stored(&b).await);
    let state = f.state(&p).await;
    assert_eq!(state.votes.len(), 1);
    assert_eq!(state.close_reason.as_deref(), Some("first"));
    let winner = if a_result.is_ok() { &a } else { &b };
    assert!(f.accept(winner).await.unwrap().unwrap().is_empty());
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM events WHERE community_id=$1 AND kind=$2 AND deleted_at IS NULL",
    )
    .bind(f.community.as_uuid())
    .bind(KIND_INTERACTION_STATE as i32)
    .fetch_one(&f.db.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn rejected_answers_are_not_stored_and_roles_are_live() {
    let f = Fixture::new().await;
    let p = f.prompt("manual", "role:owner");
    f.accept(&p).await.unwrap();
    let a = f.answer(&f.alice, &p, "yes");
    assert!(f.accept(&a).await.is_err());
    assert!(!f.stored(&a).await);
    sqlx::query("INSERT INTO relay_members(community_id,pubkey,role) VALUES($1,$2,'owner')")
        .bind(f.community.as_uuid())
        .bind(f.alice.public_key().to_hex())
        .execute(&f.db.pool)
        .await
        .unwrap();
    f.accept(&a).await.unwrap();
    let stranger = f.answer(&Keys::generate(), &p, "yes");
    assert!(f.accept(&stranger).await.is_err());
    let invalid = f.answer(&f.alice, &p, "maybe");
    assert!(f.accept(&invalid).await.is_err());
    assert!(!f.stored(&invalid).await);
    let other = Fixture::new().await;
    assert!(other
        .db
        .accept_interaction(other.community, &a, &other.relay, None)
        .await
        .is_err());
    let close = f.event(
        &f.bob,
        KIND_INTERACTION_CLOSE,
        "",
        vec![vec!["e", &p.id.to_hex(), "", "prompt"]],
    );
    assert!(f.accept(&close).await.is_err());
    sqlx::query("UPDATE channel_members SET removed_at=now() WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3").bind(f.community.as_uuid()).bind(f.channel).bind(f.alice.public_key().to_bytes().as_slice()).execute(&f.db.pool).await.unwrap();
    let changed = f.answer(&f.alice, &p, "no");
    assert!(f.accept(&changed).await.is_err());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn text_fallback_preserves_signed_source_and_actor_deduplication() {
    let f = Fixture::new().await;
    let p = f.prompt("manual", "members");
    let rows = f.accept(&p).await.unwrap().unwrap();
    let projection = rows
        .iter()
        .find(|e| e.event.kind.as_u16() as u32 == KIND_STREAM_MESSAGE)
        .unwrap();
    let reply = f.event(
        &f.alice,
        KIND_STREAM_MESSAGE,
        " YES \n",
        vec![vec!["e", &projection.event.id.to_hex(), "", "reply"]],
    );
    let accepted = f.accept(&reply).await.unwrap().unwrap();
    let synthetic = accepted
        .iter()
        .find(|e| e.event.kind.as_u16() as u32 == KIND_INTERACTION_RESPONSE)
        .unwrap();
    assert_eq!(synthetic.event.pubkey, f.relay.public_key());
    synthetic.event.verify().unwrap();
    assert_eq!(
        interaction::single_tag(&synthetic.event, "via").unwrap(),
        Some(reply.id.to_hex().as_str())
    );
    assert_eq!(
        f.state(&p).await.votes[&f.alice.public_key().to_hex()].source,
        reply
    );
    let prose = f.event(
        &f.bob,
        KIND_STREAM_MESSAGE,
        "yes but revise",
        vec![vec!["e", &projection.event.id.to_hex(), "", "reply"]],
    );
    assert!(f.accept(&prose).await.unwrap().is_none());
    let later = EventBuilder::new(Kind::Custom(KIND_INTERACTION_RESPONSE as u16), "")
        .tags(f.answer(&f.alice, &p, "no").tags.to_vec())
        .custom_created_at(Timestamp::from(reply.created_at.as_secs() + 1))
        .sign_with_keys(&f.alice)
        .unwrap();
    f.accept(&later).await.unwrap();
    assert_eq!(f.state(&p).await.votes.len(), 1);
    assert_eq!(
        f.state(&p).await.summary(&Prompt::parse(&p).unwrap())["tally"]["yes"],
        0
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn state_failure_rolls_back_response_and_tally() {
    let f = Fixture::new().await;
    let p = f.prompt("manual", "members");
    f.accept(&p).await.unwrap();
    // A newer authoritative coordinate forces the production replacement seam
    // to reject our next state. The answer INSERT must roll back with it.
    let schema = Prompt::parse(&p).unwrap();
    let future = InteractionState::default()
        .event_builder(p.id, &schema, Timestamp::now().as_secs() + 100)
        .unwrap()
        .sign_with_keys(&f.relay)
        .unwrap();
    f.db.replace_parameterized_event(f.community, &future, &p.id.to_hex(), Some(f.channel))
        .await
        .unwrap();
    let answer = f.answer(&f.alice, &p, "yes");
    assert!(f.accept(&answer).await.is_err());
    assert!(!f.stored(&answer).await);
    assert!(f.state(&p).await.votes.is_empty());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn expiry_is_durable_and_rejects_answers_before_sweep() {
    let f = Fixture::new().await;
    let base = f.prompt("manual", "members");
    let mut tags = base.tags.clone().to_vec();
    tags.retain(|t| t.kind().to_string() != "deadline");
    tags.push(Tag::parse(["deadline", &(Timestamp::now().as_secs() + 2).to_string()]).unwrap());
    let p = EventBuilder::new(base.kind, base.content)
        .tags(tags)
        .sign_with_keys(&f.asker)
        .unwrap();
    f.accept(&p).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let answer = f.answer(&f.alice, &p, "yes");
    assert!(f.accept(&answer).await.is_err());
    assert!(!f.stored(&answer).await);
    assert!(f.db.expire_interactions(&f.relay).await.unwrap() >= 1);
    assert_eq!(f.state(&p).await.close_reason.as_deref(), Some("expiry"));
    assert_eq!(f.db.expire_interactions(&f.relay).await.unwrap(), 0);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn outbox_retains_unacknowledged_work_and_discards_removed_events() {
    let f = Fixture::new().await;
    let p = f.prompt("first", "members");
    let initial = f.accept(&p).await.unwrap().unwrap();
    let initial_state = initial
        .iter()
        .find(|e| e.event.kind.as_u16() as u32 == KIND_INTERACTION_STATE)
        .unwrap();
    let answer = f.answer(&f.alice, &p, "yes");
    f.accept(&answer).await.unwrap();
    f.db.soft_delete_event(f.community, p.id.as_bytes())
        .await
        .unwrap();
    let pending = f.db.pending_interaction_events().await.unwrap();
    let ours: Vec<_> = pending
        .iter()
        .filter(|e| e.community == f.community)
        .collect();
    assert!(ours.iter().any(|e| e.event.id == answer.id));
    assert!(!ours
        .iter()
        .any(|e| e.event.id == p.id || e.event.id == initial_state.event.id));
    let repeated = f.db.pending_interaction_events().await.unwrap();
    assert!(repeated
        .iter()
        .any(|e| e.community == f.community && e.event.id == answer.id));
    f.db.acknowledge_interaction_event(f.community, &answer)
        .await
        .unwrap();
    assert!(!f
        .db
        .pending_interaction_events()
        .await
        .unwrap()
        .iter()
        .any(|e| e.community == f.community && e.event.id == answer.id));
}
