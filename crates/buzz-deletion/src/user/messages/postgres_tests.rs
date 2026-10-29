use super::*;
use buzz_core::channel::{ChannelType, ChannelVisibility};
use buzz_db::entity_deletion::{inventory_digest, NewEntityStage, NextStage, StageTransition};
use buzz_db::event::ThreadMetadataParams;
use buzz_db::user_deletion_fences::{self, FenceIdentity};
use chrono::{DateTime, Utc};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use serde_json::json;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

struct Fixture {
    db: Db,
    community: CommunityId,
    channel: Uuid,
    alice: Keys,
    bob: Keys,
    relay: Keys,
    lease: EntityLease,
    stage: EntityStage,
}

fn database_url() -> String {
    std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap()
}

async fn fixture() -> Fixture {
    let url = database_url();
    let db = Db::from_pool(PgPool::connect(&url).await.unwrap());
    let community = db
        .ensure_configured_community(&format!("messages-{}.example", Uuid::new_v4()))
        .await
        .unwrap()
        .id;
    let alice = Keys::generate();
    let bob = Keys::generate();
    let relay = Keys::generate();
    let channel = Uuid::new_v4();
    db.create_channel_with_id(
        community,
        channel,
        "messages",
        ChannelType::Stream,
        ChannelVisibility::Private,
        None,
        &bob.public_key().to_bytes(),
        None,
    )
    .await
    .unwrap();
    let target = alice.public_key().to_hex();
    let item =
        json!({"community_id": community.as_uuid(), "relationship":"participant", "label":"test"});
    let inventory = json!({"communities":[item]});
    let digest = inventory_digest(&inventory).unwrap();
    let request: Uuid = sqlx::query_scalar(
        "INSERT INTO entity_deletion_requests (target,requested_by,status,inventory_manifest,inventory_digest)
         VALUES ($1,'submitter','inventoried',$2,$3) RETURNING id",
    ).bind(&target).bind(&inventory).bind(&digest).fetch_one(db.pool()).await.unwrap();
    let store = db.entity_deletion_store();
    store.approve(request, "approver", &digest).await.unwrap();
    let lease = store
        .claim(request, "messages-test", Duration::from_secs(600))
        .await
        .unwrap()
        .unwrap();
    let NextStage::Stage(stage) = store
        .transition_stage(&lease, None, |_, _| {
            Ok(StageTransition::Initialize(NewEntityStage {
                stage: "purge_messages".into(),
                community: Some(*community.as_uuid()),
                inputs: json!({"target":target,"community":item}),
            }))
        })
        .await
        .unwrap()
    else {
        panic!("stage expected")
    };
    let mut tx = db.pool().begin().await.unwrap();
    user_deletion_fences::insert(
        &mut tx,
        FenceIdentity::user(alice.public_key()),
        &lease,
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    Fixture {
        db,
        community,
        channel,
        alice,
        bob,
        relay,
        lease,
        stage,
    }
}

fn tag(parts: &[&str]) -> Tag {
    Tag::parse(parts.iter().copied()).unwrap()
}
fn timestamp(e: &Event) -> DateTime<Utc> {
    DateTime::from_timestamp(e.created_at.as_secs() as i64, 0).unwrap()
}

impl Fixture {
    async fn event(
        &self,
        who: &Keys,
        kind: u16,
        text: &str,
        mut tags: Vec<Tag>,
        thread: Option<(&Event, &Event, i32)>,
    ) -> Event {
        tags.push(tag(&["h", &self.channel.to_string()]));
        if let Some((root, parent, _)) = thread {
            tags.push(tag(&["e", &root.id.to_hex(), "", "root"]));
            tags.push(tag(&["e", &parent.id.to_hex(), "", "reply"]));
        }
        let event = EventBuilder::new(Kind::Custom(kind), text)
            .tags(tags)
            .sign_with_keys(who)
            .unwrap();
        let metadata = if [9, 40002, 45001, 45003].contains(&kind) {
            Some(ThreadMetadataParams {
                event_id: event.id.as_bytes(),
                event_created_at: timestamp(&event),
                channel_id: self.channel,
                parent_event_id: thread.map(|(_, p, _)| p.id.as_bytes().as_slice()),
                parent_event_created_at: thread.map(|(_, p, _)| timestamp(p)),
                root_event_id: thread.map(|(r, _, _)| r.id.as_bytes().as_slice()),
                root_event_created_at: thread.map(|(r, _, _)| timestamp(r)),
                depth: thread.map_or(0, |(_, _, d)| d),
                broadcast: false,
            })
        } else {
            None
        };
        self.db
            .insert_event_with_thread_metadata(self.community, &event, Some(self.channel), metadata)
            .await
            .unwrap();
        event
    }
    async fn batch(&self, size: u16) -> Result<MessageBatch> {
        handle(
            &self.db,
            &self.lease,
            &self.stage,
            &self.relay.public_key().to_bytes(),
            size,
        )
        .await
    }
    async fn finish(&self, size: u16) {
        for _ in 0..30 {
            if self.batch(size).await.unwrap().exhausted {
                return;
            }
        }
        panic!("handler did not exhaust bounded fixture");
    }
    async fn exists(&self, e: &Event) -> bool {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND id=$2)")
            .bind(self.community.as_uuid())
            .bind(e.id.as_bytes().as_slice())
            .fetch_one(self.db.pool())
            .await
            .unwrap()
    }
    async fn reaction(&self, actor: &Keys, target: &Event) -> Event {
        let e = self
            .event(actor, 7, "👍", vec![tag(&["e", &target.id.to_hex()])], None)
            .await;
        sqlx::query("INSERT INTO reactions (community_id,event_created_at,event_id,pubkey,emoji,reaction_event_id) VALUES ($1,$2,$3,$4,'👍',$5)")
            .bind(self.community.as_uuid()).bind(timestamp(target)).bind(target.id.as_bytes().as_slice())
            .bind(actor.public_key().to_bytes().as_slice()).bind(e.id.as_bytes().as_slice()).execute(self.db.pool()).await.unwrap();
        e
    }
    async fn count(&self, query: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(query))
            .bind(self.community.as_uuid())
            .fetch_one(self.db.pool())
            .await
            .unwrap()
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn removes_messages_edits_reactions_mentions_and_delivery_but_keeps_others() {
    let f = fixture().await;
    let root = f.event(&f.bob, 9, "Bob root", vec![], None).await;
    let alice = f
        .event(
            &f.alice,
            9,
            "Alice secret",
            vec![tag(&["p", &f.bob.public_key().to_hex()])],
            Some((&root, &root, 1)),
        )
        .await;
    let bob = f
        .event(
            &f.bob,
            9,
            "Bob mentions Alice",
            vec![tag(&["p", &f.alice.public_key().to_hex()])],
            Some((&root, &alice, 2)),
        )
        .await;
    let edit1 = f
        .event(
            &f.alice,
            40003,
            "old edit secret",
            vec![tag(&["e", &alice.id.to_hex()])],
            None,
        )
        .await;
    let edit2 = f
        .event(
            &f.alice,
            40003,
            "new edit secret",
            vec![tag(&["e", &alice.id.to_hex()])],
            None,
        )
        .await;
    sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(edit1.id.as_bytes().as_slice())
        .execute(f.db.pool())
        .await
        .unwrap();
    let own_reaction = f.reaction(&f.alice, &root).await;
    let other_reaction = f.reaction(&f.bob, &alice).await;
    let kept_reaction = f.reaction(&f.bob, &root).await;
    sqlx::query("INSERT INTO push_match_queue (community_id,event_id) VALUES ($1,$2) ON CONFLICT DO NOTHING")
        .bind(f.community.as_uuid()).bind(alice.id.as_bytes().as_slice()).execute(f.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO delivery_log (community_id,event_id,success) VALUES ($1,$2,true)")
        .bind(f.community.as_uuid())
        .bind(alice.id.as_bytes().as_slice())
        .execute(f.db.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO operator_listener_outbox (listener_pubkey,target_pubkey,community_id,event_id,event_kind,event_created_at) VALUES ($1,$2,$3,$4,9,$5)")
        .bind(f.bob.public_key().to_bytes().as_slice()).bind(f.alice.public_key().to_bytes().as_slice())
        .bind(f.community.as_uuid()).bind(alice.id.as_bytes().as_slice()).bind(timestamp(&alice)).execute(f.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO push_leases (community_id,author,installation_id,source_event_id,source_created_at,generation,active,expires_at) VALUES ($1,$2,'test',$3,0,1,false,0)")
        .bind(f.community.as_uuid()).bind(f.bob.public_key().to_bytes().as_slice()).bind([88u8;32].as_slice()).execute(f.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO push_wake_outbox (community_id,author,installation_id,lease_generation,endpoint_hash,event_id,class,expires_at) VALUES ($1,$2,'test',1,$3,$4,'default',0)")
        .bind(f.community.as_uuid()).bind(f.bob.public_key().to_bytes().as_slice()).bind([89u8;32].as_slice()).bind(alice.id.as_bytes().as_slice()).execute(f.db.pool()).await.unwrap();
    f.finish(1).await;
    for event in [&alice, &edit1, &edit2, &own_reaction, &other_reaction] {
        assert!(!f.exists(event).await);
    }
    for event in [&root, &bob, &kept_reaction] {
        assert!(f.exists(event).await);
    }
    assert_eq!(
        f.count("SELECT count(*) FROM reactions WHERE community_id=$1")
            .await,
        1
    );
    assert_eq!(
        f.count("SELECT count(*) FROM event_mentions WHERE community_id=$1")
            .await,
        1
    );
    for table in [
        "push_match_queue",
        "push_wake_outbox",
        "delivery_log",
        "operator_listener_outbox",
    ] {
        assert_eq!(
            f.count(&format!(
                "SELECT count(*) FROM {table} WHERE community_id=$1"
            ))
            .await,
            0
        );
    }
    let summary =
        f.db.get_thread_summary(f.community, root.id.as_bytes())
            .await
            .unwrap()
            .unwrap();
    assert_eq!((summary.reply_count, summary.descendant_count), (0, 1));
    let intermediate =
        f.db.get_thread_metadata_by_event(f.community, alice.id.as_bytes())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(intermediate.reply_count, 1);
    let completed =
        f.db.entity_deletion_store()
            .get(f.lease.request_id)
            .await
            .unwrap();
    assert_ne!(completed.status, "completed"); // handler never advances the request
    let (status, current): (String, bool) =
        sqlx::query_as("SELECT status,is_current FROM entity_deletion_stage_history WHERE id=$1")
            .bind(f.stage.id)
            .fetch_one(f.db.pool())
            .await
            .unwrap();
    assert_eq!(status, "completed");
    assert!(current); // the central transition function still owns routing
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn purged_root_keeps_signed_replies_readable_without_retaining_its_payload() {
    let f = fixture().await;
    let root = f.event(&f.alice, 9, "private root", vec![], None).await;
    let reply = f
        .event(&f.bob, 9, "retained reply", vec![], Some((&root, &root, 1)))
        .await;
    f.finish(10).await;
    assert!(!f.exists(&root).await);
    let req=buzz_core::thread_window::Request::parse(&json!({"thread_window":true,"#h":[f.channel],"#e":[root.id.to_hex()],"kinds":[9],"limit":10})).unwrap();
    let (window, _) =
        f.db.get_thread_window_with_session(
            f.community,
            &req,
            &mut buzz_db::thread_window::ScanBudget::default(),
        )
        .await
        .unwrap();
    assert!(window.root_in_channel);
    assert_eq!(window.rows.len(), 1);
    assert_eq!(window.rows[0].event, reply);
    let mut wrong = req.clone();
    wrong.channel = Uuid::new_v4();
    let (window, _) =
        f.db.get_thread_window_with_session(
            f.community,
            &wrong,
            &mut buzz_db::thread_window::ScanBudget::default(),
        )
        .await
        .unwrap();
    assert!(!window.root_in_channel);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn attachment_in_an_old_edit_preserves_source_and_records_retry() {
    let f = fixture().await;
    let root = f.event(&f.alice, 9, "message", vec![], None).await;
    let edit = f
        .event(
            &f.alice,
            40003,
            "old attachment",
            vec![
                tag(&["e", &root.id.to_hex()]),
                tag(&["imeta", "url https://files.example/a"]),
            ],
            None,
        )
        .await;
    let error = f.batch(10).await.unwrap_err();
    assert!(error.to_string().contains("file") || error.to_string().contains("attachment"));
    assert!(f.exists(&root).await && f.exists(&edit).await);
    let (status, error): (String, Option<String>) =
        sqlx::query_as("SELECT status,last_error FROM entity_deletion_stage_history WHERE id=$1")
            .bind(f.stage.id)
            .fetch_one(f.db.pool())
            .await
            .unwrap();
    assert_eq!(status, "failed");
    assert!(error.is_some());
    assert_eq!(
        f.db.entity_deletion_store()
            .get(f.lease.request_id)
            .await
            .unwrap()
            .status,
        "retry_wait"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn database_failure_rolls_back_dependents_and_counts_then_restart_finishes() {
    let f = fixture().await;
    let root = f.event(&f.bob, 9, "root", vec![], None).await;
    let alice = f
        .event(
            &f.alice,
            9,
            "reply",
            vec![tag(&["p", &f.bob.public_key().to_hex()])],
            Some((&root, &root, 1)),
        )
        .await;
    let reaction = f.reaction(&f.bob, &alice).await;
    let trigger = format!("fail_message_purge_{}", Uuid::new_v4().simple());
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION {trigger}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected failure'; END $$;
         CREATE TRIGGER {trigger} BEFORE UPDATE ON thread_metadata FOR EACH ROW
         WHEN (NEW.community_id = '{}') EXECUTE FUNCTION {trigger}();",
        f.community.as_uuid(),
    ))).execute(f.db.pool()).await.unwrap();
    assert!(f
        .batch(10)
        .await
        .unwrap_err()
        .to_string()
        .contains("injected failure"));
    assert!(f.exists(&alice).await && f.exists(&reaction).await);
    assert_eq!(
        f.count("SELECT count(*) FROM reactions WHERE community_id=$1")
            .await,
        1
    );
    assert_eq!(
        f.count("SELECT count(*) FROM event_mentions WHERE community_id=$1")
            .await,
        1
    );
    assert_eq!(
        f.db.get_thread_summary(f.community, root.id.as_bytes())
            .await
            .unwrap()
            .unwrap()
            .reply_count,
        1
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "DROP TRIGGER {trigger} ON thread_metadata; DROP FUNCTION {trigger}();"
    )))
    .execute(f.db.pool())
    .await
    .unwrap();
    sqlx::query(
        "UPDATE entity_deletion_requests SET next_attempt_at=clock_timestamp() WHERE id=$1",
    )
    .bind(f.lease.request_id)
    .execute(f.db.pool())
    .await
    .unwrap();
    let restarted = Db::from_pool(PgPool::connect(&database_url()).await.unwrap());
    assert!(
        !handle(
            &restarted,
            &f.lease,
            &f.stage,
            &f.relay.public_key().to_bytes(),
            1
        )
        .await
        .unwrap()
        .exhausted
    );
    f.finish(1).await;
    assert!(!f.exists(&alice).await && !f.exists(&reaction).await);
    assert_eq!(
        f.db.get_thread_summary(f.community, root.id.as_bytes())
            .await
            .unwrap()
            .unwrap()
            .reply_count,
        0
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn only_trusted_relay_attribution_counts_and_other_community_survives() {
    let f = fixture().await;
    let alice = f.event(&f.alice, 9, "direct", vec![], None).await;
    let relayed = f
        .event(
            &f.relay,
            9,
            "legacy attributed",
            vec![tag(&["p", &f.alice.public_key().to_hex()])],
            None,
        )
        .await;
    let bob = f
        .event(
            &f.bob,
            9,
            "not Alice",
            vec![tag(&["actor", &f.alice.public_key().to_hex()])],
            None,
        )
        .await;
    let other =
        f.db.ensure_configured_community("other.example")
            .await
            .unwrap()
            .id;
    // Same signed event in another tenant, with no shared channel provenance.
    f.db.insert_event(other, &alice, None).await.unwrap();
    f.finish(10).await;
    assert!(!f.exists(&alice).await && !f.exists(&relayed).await);
    assert!(f.exists(&bob).await);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
            .bind(other.as_uuid())
            .bind(alice.id.as_bytes().as_slice())
            .fetch_one(f.db.pool())
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn missing_fence_wrong_inventory_and_stale_lease_cannot_delete() {
    let f = fixture().await;
    let event = f
        .event(&f.alice, 9, "keep until authorized", vec![], None)
        .await;
    f.db.entity_deletion_store()
        .start_stage(&f.lease, f.stage.id)
        .await
        .unwrap();
    let mut stale = f.lease.clone();
    stale.generation += 1;
    assert!(f
        .db
        .user_deletion_store()
        .purge_message_batch(
            &stale,
            f.stage.id,
            f.community,
            &f.relay.public_key().to_bytes(),
            1
        )
        .await
        .is_err());
    let other =
        f.db.ensure_configured_community("unapproved.example")
            .await
            .unwrap()
            .id;
    assert!(f
        .db
        .user_deletion_store()
        .purge_message_batch(
            &f.lease,
            f.stage.id,
            other,
            &f.relay.public_key().to_bytes(),
            1
        )
        .await
        .is_err());
    sqlx::query("DELETE FROM user_deletion_fences WHERE request_id=$1")
        .bind(f.lease.request_id)
        .execute(f.db.pool())
        .await
        .unwrap();
    assert!(f
        .db
        .user_deletion_store()
        .purge_message_batch(
            &f.lease,
            f.stage.id,
            f.community,
            &f.relay.public_key().to_bytes(),
            1
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("fenced"));
    assert!(f.exists(&event).await);
}

async fn workflow(f: &Fixture, trigger: &Event, status: &str) -> (Uuid, Uuid) {
    sqlx::query("INSERT INTO users (community_id,pubkey) VALUES ($1,$2) ON CONFLICT DO NOTHING")
        .bind(f.community.as_uuid())
        .bind(f.bob.public_key().to_bytes().as_slice())
        .execute(f.db.pool())
        .await
        .unwrap();
    let workflow:Uuid=sqlx::query_scalar("INSERT INTO workflows (community_id,name,owner_pubkey,definition,definition_hash) VALUES ($1,'Bob workflow',$2,'{}','\\x01') RETURNING id")
        .bind(f.community.as_uuid()).bind(f.bob.public_key().to_bytes().as_slice()).fetch_one(f.db.pool()).await.unwrap();
    let run:Uuid=sqlx::query_scalar("INSERT INTO workflow_runs (community_id,workflow_id,status,trigger_event_id,trigger_context,execution_trace) VALUES ($1,$2,$3::run_status,$4,$5,$6) RETURNING id")
        .bind(f.community.as_uuid()).bind(workflow).bind(status).bind(trigger.id.as_bytes().as_slice())
        .bind(json!({"text":"saved secret","author":f.alice.public_key().to_hex(),"message_id":trigger.id.to_hex()}))
        .bind(json!([{"output":"copied secret"}])).fetch_one(f.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO workflow_approvals (community_id,token,workflow_id,run_id,step_id,step_index,approver_spec,expires_at) VALUES ($1,'\\x01',$2,$3,'approval',0,'owner',now())")
        .bind(f.community.as_uuid()).bind(workflow).bind(run).execute(f.db.pool()).await.unwrap();
    sqlx::query("INSERT INTO scheduled_workflow_fires (community_id,workflow_id,scheduled_for,workflow_run_id) VALUES ($1,$2,now(),$3)")
        .bind(f.community.as_uuid()).bind(workflow).bind(run).execute(f.db.pool()).await.unwrap();
    (workflow, run)
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn workflow_copies_and_approvals_are_deleted_but_definition_and_cron_claim_remain() {
    let f = fixture().await;
    let event = f.event(&f.alice, 9, "workflow trigger", vec![], None).await;
    let (_workflow, _run) = workflow(&f, &event, "completed").await;
    f.finish(10).await;
    assert_eq!(
        f.count("SELECT count(*) FROM workflow_runs WHERE community_id=$1")
            .await,
        0
    );
    assert_eq!(
        f.count("SELECT count(*) FROM workflow_approvals WHERE community_id=$1")
            .await,
        0
    );
    assert_eq!(
        f.count("SELECT count(*) FROM workflows WHERE community_id=$1")
            .await,
        1
    );
    assert_eq!(f.count("SELECT count(*) FROM scheduled_workflow_fires WHERE community_id=$1 AND workflow_run_id IS NULL").await,1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn active_workflow_and_claimed_delivery_prevent_message_removal() {
    let f = fixture().await;
    let event = f.event(&f.alice, 9, "still in use", vec![], None).await;
    let (_, run) = workflow(&f, &event, "running").await;
    assert!(f
        .batch(10)
        .await
        .unwrap_err()
        .to_string()
        .contains("workflow runs"));
    assert!(f.exists(&event).await);
    sqlx::query("UPDATE workflow_runs SET status='cancelled' WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(run)
        .execute(f.db.pool())
        .await
        .unwrap();
    sqlx::query(
        "UPDATE entity_deletion_requests SET next_attempt_at=clock_timestamp() WHERE id=$1",
    )
    .bind(f.lease.request_id)
    .execute(f.db.pool())
    .await
    .unwrap();
    sqlx::query("INSERT INTO push_match_queue (community_id,event_id,state,lease_until) VALUES ($1,$2,'matching',now()+interval '1 hour') ON CONFLICT (community_id,event_id) DO UPDATE SET state='matching',lease_until=now()+interval '1 hour'")
        .bind(f.community.as_uuid()).bind(event.id.as_bytes().as_slice()).execute(f.db.pool()).await.unwrap();
    assert!(f
        .batch(10)
        .await
        .unwrap_err()
        .to_string()
        .contains("delivery workers"));
    assert!(f.exists(&event).await);
    assert_eq!(
        f.count("SELECT count(*) FROM workflow_runs WHERE community_id=$1")
            .await,
        1
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn reaction_only_threads_and_orphan_rows_are_not_missed() {
    let f = fixture().await;
    let root = f.event(&f.bob, 9, "Bob only", vec![], None).await;
    let reaction = f.reaction(&f.alice, &root).await;
    sqlx::query("INSERT INTO reactions (community_id,event_created_at,event_id,pubkey,emoji,removed_at) VALUES ($1,now(),$2,$3,'old',now())")
        .bind(f.community.as_uuid()).bind([42u8;32].as_slice()).bind(f.alice.public_key().to_bytes().as_slice()).execute(f.db.pool()).await.unwrap();
    f.finish(1).await;
    assert!(f.exists(&root).await);
    assert!(!f.exists(&reaction).await);
    assert_eq!(
        f.count("SELECT count(*) FROM reactions WHERE community_id=$1")
            .await,
        0
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn existing_soft_delete_does_not_decrement_remaining_counts_twice() {
    let f = fixture().await;
    let root = f.event(&f.bob, 9, "Bob root", vec![], None).await;
    let alice = f
        .event(
            &f.alice,
            9,
            "already hidden",
            vec![],
            Some((&root, &root, 1)),
        )
        .await;
    f.event(&f.bob, 9, "retained", vec![], Some((&root, &root, 1)))
        .await;
    sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(alice.id.as_bytes().as_slice())
        .execute(f.db.pool())
        .await
        .unwrap();
    // Seed the correct post-soft-delete counts. Hard deletion must not subtract again.
    sqlx::query("UPDATE thread_metadata SET reply_count=1,descendant_count=1 WHERE community_id=$1 AND event_id=$2")
        .bind(f.community.as_uuid()).bind(root.id.as_bytes().as_slice()).execute(f.db.pool()).await.unwrap();
    f.finish(1).await;
    let summary =
        f.db.get_thread_summary(f.community, root.id.as_bytes())
            .await
            .unwrap()
            .unwrap();
    assert_eq!((summary.reply_count, summary.descendant_count), (1, 1));
    assert!(summary.last_reply_at.is_some());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn uses_the_authoritative_target_and_preserves_other_peoples_diffs_and_pins() {
    let f = fixture().await;
    let alice = f.event(&f.alice, 45001, "Alice forum", vec![], None).await;
    let bob = f.event(&f.bob, 45001, "Bob forum", vec![], None).await;
    let keep_vote = f
        .event(
            &f.bob,
            45002,
            "+",
            vec![
                tag(&["e", &bob.id.to_hex()]),
                tag(&["e", &alice.id.to_hex()]),
            ],
            None,
        )
        .await;
    let delete_vote = f
        .event(
            &f.bob,
            45002,
            "+",
            vec![
                tag(&["e", &alice.id.to_hex()]),
                tag(&["e", &bob.id.to_hex()]),
            ],
            None,
        )
        .await;
    let keep_reaction = f
        .event(
            &f.bob,
            7,
            "+",
            vec![
                tag(&["e", &alice.id.to_hex()]),
                tag(&["e", &bob.id.to_hex()]),
            ],
            None,
        )
        .await;
    let delete_reaction = f
        .event(
            &f.bob,
            7,
            "+",
            vec![
                tag(&["e", &bob.id.to_hex()]),
                tag(&["e", &alice.id.to_hex()]),
            ],
            None,
        )
        .await;
    let diff = f
        .event(
            &f.bob,
            40008,
            "Bob diff",
            vec![tag(&["e", &alice.id.to_hex()])],
            None,
        )
        .await;
    let pin = f
        .event(
            &f.bob,
            40004,
            "Bob pin",
            vec![tag(&["e", &alice.id.to_hex()])],
            None,
        )
        .await;
    f.finish(10).await;
    for event in [&alice, &delete_vote, &delete_reaction] {
        assert!(!f.exists(event).await);
    }
    for event in [&bob, &keep_vote, &keep_reaction, &diff, &pin] {
        assert!(f.exists(event).await);
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn legacy_messages_without_metadata_and_fully_removed_threads_exhaust() {
    let f = fixture().await;
    let root = f.event(&f.alice, 9, "root", vec![], None).await;
    let reply = f
        .event(&f.alice, 9, "legacy reply", vec![], Some((&root, &root, 1)))
        .await;
    sqlx::query("DELETE FROM thread_metadata WHERE community_id=$1 AND event_id=$2")
        .bind(f.community.as_uuid())
        .bind(reply.id.as_bytes().as_slice())
        .execute(f.db.pool())
        .await
        .unwrap();
    f.finish(1).await;
    assert!(!f.exists(&root).await && !f.exists(&reply).await);
    assert_eq!(
        f.count("SELECT count(*) FROM thread_metadata WHERE community_id=$1")
            .await,
        0
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn saved_workflow_author_finds_copies_after_the_source_event_is_gone() {
    let f = fixture().await;
    let source = f.event(&f.alice, 9, "old source", vec![], None).await;
    workflow(&f, &source, "completed").await;
    sqlx::query("DELETE FROM events WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid())
        .bind(source.id.as_bytes().as_slice())
        .execute(f.db.pool())
        .await
        .unwrap();
    f.finish(1).await;
    assert_eq!(
        f.count("SELECT count(*) FROM workflow_runs WHERE community_id=$1")
            .await,
        0
    );
}
