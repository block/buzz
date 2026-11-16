use super::*;
use buzz_db::entity_deletion::{NewEntityStage, NextStage, StageTransition};
use buzz_db::user_deletion::{agents::AgentsDbProgress, UserDeletionInventory};
use buzz_db::user_deletion_fences::{self, FenceIdentity};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde_json::Value;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

mod recovery_postgres_tests;
mod safety_postgres_tests;

struct Fixture {
    db: Db,
    community: CommunityId,
    owner: Keys,
    agents: Vec<Keys>,
    other: Keys,
    relay: Keys,
    lease: EntityLease,
    stage: EntityStage,
}

fn database_url() -> String {
    std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap()
}

async fn fixture(count: usize) -> Fixture {
    let db = Db::from_pool(PgPool::connect(&database_url()).await.unwrap());
    let community = db
        .ensure_configured_community(&format!("agents-{}.example", Uuid::new_v4()))
        .await
        .unwrap()
        .id;
    let owner = Keys::generate();
    let other = Keys::generate();
    let relay = Keys::generate();
    let mut agents: Vec<_> = (0..count).map(|_| Keys::generate()).collect();
    agents.sort_by_key(|key| key.public_key().to_hex());
    for key in [&owner, &other].into_iter().chain(agents.iter()) {
        sqlx::query("INSERT INTO users (community_id,pubkey,display_name) VALUES ($1,$2,'retained profile')")
            .bind(community.as_uuid()).bind(key.public_key().to_bytes().as_slice()).execute(db.pool()).await.unwrap();
    }
    for agent in &agents {
        sqlx::query("UPDATE users SET agent_owner_pubkey=$3 WHERE community_id=$1 AND pubkey=$2")
            .bind(community.as_uuid())
            .bind(agent.public_key().to_bytes().as_slice())
            .bind(owner.public_key().to_bytes().as_slice())
            .execute(db.pool())
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO relay_members (community_id,pubkey,role) VALUES ($1,$2,'member')")
        .bind(community.as_uuid())
        .bind(owner.public_key().to_hex())
        .execute(db.pool())
        .await
        .unwrap();
    let users = db.user_deletion_store();
    let request = users
        .submit(&owner.public_key().to_hex(), "submitter", None)
        .await
        .unwrap();
    let inventory = UserDeletionInventory::read(&request).unwrap();
    assert_eq!(
        inventory
            .agents(*community.as_uuid(), &request.target)
            .unwrap(),
        agents
            .iter()
            .map(|key| key.public_key().to_hex())
            .collect::<Vec<_>>()
    );
    users
        .approve(
            request.id,
            "approver",
            request.inventory_digest.as_deref().unwrap(),
        )
        .await
        .unwrap();
    let store = db.entity_deletion_store();
    let lease = store
        .claim(request.id, "agent-worker", Duration::from_secs(600))
        .await
        .unwrap()
        .unwrap();
    let NextStage::Stage(stage) = store
        .transition_stage(&lease, None, |_, _| {
            Ok(StageTransition::Initialize(NewEntityStage {
                stage: "purge_agents_db".into(),
                community: Some(*community.as_uuid()),
                inputs: inventory.inputs(&request.target, Some(*community.as_uuid()))?,
            }))
        })
        .await
        .unwrap()
    else {
        panic!("expected stage")
    };
    let mut tx = db.pool().begin().await.unwrap();
    user_deletion_fences::insert(
        &mut tx,
        FenceIdentity::user(owner.public_key()),
        &lease,
        None,
    )
    .await
    .unwrap();
    for agent in &agents {
        user_deletion_fences::insert(
            &mut tx,
            FenceIdentity::agent(community, agent.public_key()),
            &lease,
            Some(owner.public_key()),
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    Fixture {
        db,
        community,
        owner,
        agents,
        other,
        relay,
        lease,
        stage,
    }
}

impl Fixture {
    async fn event(&self, key: &Keys, kind: u16, content: &str, tags: Vec<Tag>) -> Event {
        let event = EventBuilder::new(Kind::Custom(kind), content)
            .tags(tags)
            .sign_with_keys(key)
            .unwrap();
        self.db
            .insert_event(self.community, &event, None)
            .await
            .unwrap();
        event
    }
    async fn exists(&self, event: &Event) -> bool {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND id=$2)")
            .bind(self.community.as_uuid())
            .bind(event.id.as_bytes().as_slice())
            .fetch_one(self.db.pool())
            .await
            .unwrap()
    }
    async fn batch(&self) -> Result<AgentsDbBatch> {
        handle(
            &self.db,
            &self.lease,
            &self.stage,
            &self.relay.public_key().to_bytes(),
            1,
        )
        .await
    }
    async fn progress(&self) -> AgentsDbProgress {
        let value: Value = sqlx::query_scalar(
            "SELECT metadata_json->'progress' FROM entity_deletion_stage_history WHERE id=$1",
        )
        .bind(self.stage.id)
        .fetch_one(self.db.pool())
        .await
        .unwrap();
        serde_json::from_value(value).unwrap()
    }
    async fn finish(&self) -> AgentsDbBatch {
        for _ in 0..150 {
            let result = self.batch().await.unwrap();
            if result.exhausted {
                return result;
            }
        }
        panic!("agent fixture did not finish")
    }
    async fn due(&self) {
        sqlx::query(
            "UPDATE entity_deletion_requests SET next_attempt_at=clock_timestamp() WHERE id=$1",
        )
        .bind(self.lease.request_id)
        .execute(self.db.pool())
        .await
        .unwrap();
    }
    async fn reclaim(&mut self) {
        sqlx::query("UPDATE entity_deletion_requests SET lease_expires_at=clock_timestamp()-interval '1 second',next_attempt_at=NULL WHERE id=$1")
            .bind(self.lease.request_id).execute(self.db.pool()).await.unwrap();
        self.lease = self
            .db
            .entity_deletion_store()
            .claim(
                self.lease.request_id,
                "agent-worker",
                Duration::from_secs(600),
            )
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cleans_two_agents_and_tracks_completion_without_removing_profiles_or_neighbors() {
    let f = fixture(2).await;
    let mut deleted = Vec::new();
    let mut kept = Vec::new();
    for key in &f.agents {
        for kind in [9, 30174, 44200, 43004, 30078, 30300, 30350, 30620] {
            deleted.push(f.event(key, kind, "agent payload", vec![]).await);
        }
        for kind in [0, 10100, 30030] {
            kept.push(f.event(key, kind, "profile or shared emoji", vec![]).await);
        }
        sqlx::query("INSERT INTO personal_read_accounts (community_id,actor) VALUES ($1,$2)")
            .bind(f.community.as_uuid())
            .bind(key.public_key().to_bytes().as_slice())
            .execute(f.db.pool())
            .await
            .unwrap();
        sqlx::query("INSERT INTO subscriptions (community_id,id,owner_pubkey,delivery_url) VALUES ($1,$2,$3,'https://example.test/keep-until-drained')")
            .bind(f.community.as_uuid()).bind(key.public_key().to_hex()).bind(key.public_key().to_bytes().as_slice())
            .execute(f.db.pool()).await.unwrap();
        sqlx::query(
            "INSERT INTO delivery_log (community_id,subscription_id,event_id) VALUES ($1,$2,$3)",
        )
        .bind(f.community.as_uuid())
        .bind(key.public_key().to_hex())
        .bind([77u8; 32].as_slice())
        .execute(f.db.pool())
        .await
        .unwrap();
    }
    kept.push(f.event(&f.owner, 9, "owner message", vec![]).await);
    kept.push(f.event(&f.other, 9, "unrelated agent", vec![]).await);
    kept.push(
        f.event(
            &f.owner,
            30179,
            "private agent config",
            vec![Tag::parse(["d", &f.agents[0].public_key().to_hex()]).unwrap()],
        )
        .await,
    );
    let other_community =
        f.db.ensure_configured_community("agent-other.example")
            .await
            .unwrap()
            .id;
    f.db.insert_event(other_community, &deleted[0], None)
        .await
        .unwrap();
    let done = f.finish().await;
    assert_eq!(done.progress.agents.len(), 2);
    for agent in done.progress.agents.values() {
        assert_eq!(agent.status, AgentDbStatus::Completed);
        assert!(agent.completed_at.is_some());
        assert_eq!(agent.phase, AgentDbPhase::VerifyDbContent);
    }
    for event in deleted {
        assert!(!f.exists(&event).await);
    }
    for event in kept {
        assert!(f.exists(&event).await);
    }
    for (table, expected) in [
        ("users", 4),
        ("personal_read_accounts", 0),
        ("subscriptions", 0),
        ("delivery_log", 0),
    ] {
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {table} WHERE community_id=$1"
        )))
        .bind(f.community.as_uuid())
        .fetch_one(f.db.pool())
        .await
        .unwrap();
        assert_eq!(count, expected, "{table}");
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1")
        .bind(other_community.as_uuid())
        .fetch_one(f.db.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
    let (status,current,result):(String,bool,Value)=sqlx::query_as("SELECT status,is_current,metadata_json->'result' FROM entity_deletion_stage_history WHERE id=$1")
        .bind(f.stage.id).fetch_one(f.db.pool()).await.unwrap();
    assert_eq!(status, "completed");
    assert!(current);
    assert_eq!(result["agents_completed"], 2);
    assert_ne!(
        f.db.entity_deletion_store()
            .get(f.lease.request_id)
            .await
            .unwrap()
            .status,
        "completed"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn failed_second_agent_preserves_first_completion_and_resumes_after_same_owner_reclaim() {
    let mut f = fixture(2).await;
    let first = f.agents[0].public_key().to_hex();
    let second = f.agents[1].public_key().to_hex();
    let media = f
        .event(&f.agents[1], 9, "https://example.test/image", vec![])
        .await;
    for _ in 0..40 {
        if f.batch().await.is_err() {
            break;
        }
    }
    let progress = f.progress().await;
    assert_eq!(progress.agents[&first].status, AgentDbStatus::Completed);
    assert_eq!(progress.agents[&second].status, AgentDbStatus::Failed);
    assert_eq!(progress.current_agent, Some(second.clone()));
    assert!(
        progress.agents[&second]
            .last_error
            .as_deref()
            .unwrap()
            .contains("file")
            || progress.agents[&second]
                .last_error
                .as_deref()
                .unwrap()
                .contains("attachment")
    );
    assert!(f.exists(&media).await);
    assert!(f.batch().await.unwrap_err().to_string().contains("not due"));
    let old = f.lease.clone();
    let completed_at = progress.agents[&first].completed_at;
    // Simulate the separate media stage resolving this blocker, not the handler bypassing it.
    sqlx::query("DELETE FROM events WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(media.id.as_bytes().as_slice())
        .execute(f.db.pool())
        .await
        .unwrap();
    f.reclaim().await;
    assert!(f.lease.generation > old.generation);
    // Keep every other guard valid: same owner, live expiry, approved scope and
    // a running current stage. Only the old generation may reject these writes.
    f.db.entity_deletion_store()
        .start_stage(&f.lease, f.stage.id)
        .await
        .unwrap();
    assert!(
        handle(&f.db, &old, &f.stage, &f.relay.public_key().to_bytes(), 1)
            .await
            .is_err()
    );
    assert!(f
        .db
        .user_deletion_store()
        .fail_agents_db_stage(&old, f.stage.id, "stale error")
        .await
        .is_err());
    assert!(f
        .db
        .user_deletion_store()
        .purge_agents_db_batch(
            &old,
            f.stage.id,
            f.community,
            &f.relay.public_key().to_bytes(),
            1,
            |_, _| {},
        )
        .await
        .is_err());
    f.db = Db::from_pool(PgPool::connect(&database_url()).await.unwrap());
    let done = f.finish().await;
    assert_eq!(done.progress.agents[&first].completed_at, completed_at);
    assert_eq!(
        done.progress.agents[&second].status,
        AgentDbStatus::Completed
    );
    assert!(done.progress.agents[&second].last_error.is_none());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn empty_approved_agent_list_finishes_without_changing_other_data() {
    let f = fixture(0).await;
    let event = f.event(&f.owner, 30174, "owner memory", vec![]).await;
    let result = f.batch().await.unwrap();
    assert!(result.exhausted);
    assert!(result.progress.agents.is_empty());
    assert!(f.exists(&event).await);
}
