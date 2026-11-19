use super::scrub_postgres_tests::{options, setup, PauseAfterPlan};
use super::*;
use crate::{user_deletion::ExecutionMode, CommunityId, Db};
use nostr::{EventBuilder, Keys, Kind, Tag};

pub(super) async fn definition(db: &Db, c: CommunityId, id: Uuid, owner: &Keys) {
    sqlx::query(
        "INSERT INTO workflows(community_id,id,name,owner_pubkey,definition,definition_hash)
        VALUES($1,$2,'personal automation',$3,'{}',$4)",
    )
    .bind(c.as_uuid())
    .bind(id)
    .bind(owner.public_key().as_bytes().as_slice())
    .bind(vec![0u8; 32])
    .execute(db.pool())
    .await
    .unwrap();
}

async fn run(
    db: &Db,
    c: CommunityId,
    workflow: Uuid,
    source: Option<&str>,
    caller: Option<&Keys>,
    context: Value,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO workflow_runs(community_id,id,workflow_id,status,cause_source,cause_pubkey,trigger_context,execution_trace,error_message)
        VALUES($1,$2,$3,'completed',$4,$5,$6,'[{\"body\":\"external response\"}]','retained error')")
        .bind(c.as_uuid()).bind(id).bind(workflow).bind(source)
        .bind(caller.map(|k|k.public_key().as_bytes().to_vec())).bind(context)
        .execute(db.pool()).await.unwrap();
    id
}

async fn state(db: &Db, c: CommunityId, table: &str) -> Vec<Value> {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT to_jsonb(t) FROM {table} t WHERE community_id=$1 ORDER BY to_jsonb(t)"
    )))
    .bind(c.as_uuid())
    .fetch_all(db.pool())
    .await
    .unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn personal_workflow_cleanup_preserves_history_and_other_subjects_on_retry() {
    let (db, c, _, alice, bob) = setup().await;
    let agent = Keys::generate();
    db.ensure_user(c, agent.public_key().as_bytes())
        .await
        .unwrap();
    sqlx::query("UPDATE users SET agent_owner_pubkey=$2 WHERE community_id=$1 AND pubkey=$3")
        .bind(c.as_uuid())
        .bind(alice.public_key().as_bytes().as_slice())
        .bind(agent.public_key().as_bytes().as_slice())
        .execute(db.pool())
        .await
        .unwrap();
    let other = db
        .ensure_configured_community(&format!("workflow-other-{}.example", Uuid::new_v4()))
        .await
        .unwrap()
        .id;
    let empty = Uuid::new_v4();
    let shared = Uuid::new_v4();
    let context = json!({"text":"private trigger","emoji":"private emoji","author":alice.public_key().to_hex(),
        "webhook_fields":{"input":"manual input"},"channel_id":"retained","message_id":"retained","timestamp":"retained","is_reply":false});
    let mut scrub_inputs = HashSet::new();
    let mut scrub_notes = HashSet::new();
    for community in [c, other] {
        for author in [&alice, &bob, &agent] {
            db.ensure_user(community, author.public_key().as_bytes())
                .await
                .unwrap();
        }
        definition(&db, community, empty, &alice).await;
        definition(&db, community, shared, &alice).await;
        let others = Uuid::new_v4();
        definition(&db, community, others, &bob).await;
        for workflow in [empty, shared, others] {
            sqlx::query("INSERT INTO scheduled_workflow_fires(community_id,workflow_id,scheduled_for) VALUES($1,$2,now())")
                .bind(community.as_uuid()).bind(workflow).execute(db.pool()).await.unwrap();
        }
        // Ownership of a run's parent is irrelevant to the input's author.
        for (source, caller, payload, scrub) in [
            (Some("event"), Some(&alice), context.clone(), true),
            (Some("manual"), Some(&alice), context.clone(), true),
            (Some("event"), Some(&agent), context.clone(), true),
            (Some("event"), Some(&bob), context.clone(), false),
            (Some("manual"), Some(&bob), context.clone(), false),
            (Some("webhook"), Some(&alice), context.clone(), false),
            (Some("schedule"), Some(&alice), context.clone(), false),
            (None, None, context.clone(), false),
            (Some("event"), None, context.clone(), false),
            (Some("event"), Some(&alice), json!(["legacy"]), false),
            (
                Some("manual"),
                Some(&alice),
                json!({"webhook_fields":"legacy"}),
                false,
            ),
        ] {
            let id = run(&db, community, others, source, caller, payload).await;
            if community == c && scrub {
                scrub_inputs.insert(id.to_string());
            }
        }
        let id = run(&db, community, shared, None, None, context.clone()).await;
        // Independent foreign keys allow history linked to another parent's run.
        let approval_only = Uuid::new_v4();
        let linked_only = Uuid::new_v4();
        definition(&db, community, approval_only, &alice).await;
        definition(&db, community, linked_only, &alice).await;
        sqlx::query("INSERT INTO workflow_approvals(community_id,token,workflow_id,run_id,step_id,step_index,approver_spec,status,expires_at)
            VALUES($1,$2,$3,$4,'review',0,'reviewers','pending',now()+interval '1 day')")
            .bind(community.as_uuid()).bind(Uuid::new_v4().as_bytes().to_vec()).bind(approval_only).bind(id)
            .execute(db.pool()).await.unwrap();
        sqlx::query("INSERT INTO scheduled_workflow_fires(community_id,workflow_id,scheduled_for,workflow_run_id) VALUES($1,$2,now(),$3)")
            .bind(community.as_uuid()).bind(linked_only).bind(id).execute(db.pool()).await.unwrap();
        for workflow in [empty, shared] {
            let event = EventBuilder::new(Kind::Custom(30620), "signed workflow publication")
                .tags([Tag::parse(["d", &workflow.to_string()]).unwrap()])
                .sign_with_keys(&alice)
                .unwrap();
            db.insert_event(community, &event, None).await.unwrap();
        }
        sqlx::query("UPDATE workflow_runs SET status='waiting_approval' WHERE community_id=$1 AND cause_source='manual'")
            .bind(community.as_uuid()).execute(db.pool()).await.unwrap();
        for author in [&alice, &agent, &bob] {
            let token = Uuid::new_v4().as_bytes().to_vec();
            sqlx::query("INSERT INTO workflow_approvals(community_id,token,workflow_id,run_id,step_id,step_index,approver_spec,status,approver_pubkey,note,expires_at)
                VALUES($1,$2,$3,$4,'review',0,'reviewers','granted',$5,'personal approval note',now()+interval '1 day')")
                .bind(community.as_uuid()).bind(&token).bind(shared).bind(id)
                .bind(author.public_key().as_bytes().as_slice()).execute(db.pool()).await.unwrap();
            if community == c && author.public_key() != bob.public_key() {
                scrub_notes.insert(format!("\\x{}", hex::encode(token)));
            }
        }
    }
    // Existing signed-event cleanup still removes publications, independently
    // of the newly supported database-row policy.
    let tables = [
        "workflows",
        "workflow_runs",
        "workflow_approvals",
        "scheduled_workflow_fires",
    ];
    let mut before = Vec::new();
    let mut foreign = Vec::new();
    for table in tables {
        before.push(state(&db, c, table).await);
        foreign.push(state(&db, other, table).await);
    }
    let store = db.user_deletion_store();
    let mut opts = options(c, &alice);
    opts.batch_size = 3;
    let mut output = Vec::new();
    let preview = store
        .run_message_batches(&opts, ExecutionMode::ReadOnly, None, &mut output)
        .await
        .unwrap();
    assert_eq!(preview.items, 9);
    assert!(!String::from_utf8(output)
        .unwrap()
        .contains("personal approval note"));
    for (i, table) in tables.iter().enumerate() {
        assert_eq!(state(&db, c, table).await, before[i]);
    }
    let error = store
        .run_message_batches(
            &opts,
            ExecutionMode::Delete,
            None,
            &mut super::private_state_postgres_tests::InterruptAfterCommit(Vec::new()),
        )
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("simulated lost output"),
        "{error}"
    );
    let resumed = store
        .run_message_batches(&opts, ExecutionMode::Delete, None, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(resumed.items + 3, preview.items);
    for (i, table) in tables.iter().enumerate() {
        let mut expected = before[i].clone();
        expected.retain(|row| match *table {
            "workflows" => row["id"] != empty.to_string(),
            "scheduled_workflow_fires" => row["workflow_id"] != empty.to_string(),
            _ => true,
        });
        for row in &mut expected {
            if *table == "workflow_runs" && scrub_inputs.contains(row["id"].as_str().unwrap()) {
                if row["cause_source"] == "event" {
                    row["trigger_context"]["text"] = json!("");
                    row["trigger_context"]["emoji"] = json!("");
                } else {
                    row["trigger_context"]["webhook_fields"] = json!({});
                }
            }
            if *table == "workflow_approvals"
                && scrub_notes.contains(row["token"].as_str().unwrap())
            {
                row["note"] = Value::Null;
            }
        }
        // jsonb order can change after a field is cleared; compare full rows as sets.
        let as_set = |v: Vec<Value>| v.into_iter().map(|r| r.to_string()).collect::<HashSet<_>>();
        assert_eq!(
            as_set(state(&db, c, table).await),
            as_set(expected),
            "{table}"
        );
        assert_eq!(state(&db, other, table).await, foreign[i]);
    }
    let remaining_events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND kind=30620")
            .bind(c.as_uuid())
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(remaining_events, 0);
    assert_eq!(
        store
            .run_message_batches(&opts, ExecutionMode::Delete, None, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Postgres"]
async fn workflow_history_arriving_after_plan_prevents_cascade_and_claim_deletion() {
    let (db, c, _, alice, _) = setup().await;
    for with_claim in [false, true] {
        let workflow = Uuid::new_v4();
        definition(&db, c, workflow, &alice).await;
        if with_claim {
            sqlx::query("INSERT INTO scheduled_workflow_fires(community_id,workflow_id,scheduled_for) VALUES($1,$2,now())")
                .bind(c.as_uuid()).bind(workflow).execute(db.pool()).await.unwrap();
        }
        let (planned_tx, planned_rx) = tokio::sync::oneshot::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let mut opts = options(c, &alice);
        opts.batch_size = 2;
        let store = db.user_deletion_store();
        let task = tokio::spawn(async move {
            let mut writer = PauseAfterPlan {
                record: "delete_start",
                line: Vec::new(),
                planned: Some(planned_tx),
                resume: resume_rx,
            };
            store
                .run_message_batches(&opts, ExecutionMode::Delete, None, &mut writer)
                .await
        });
        planned_rx.await.unwrap();
        run(&db, c, workflow, None, None, json!({"external":"keep"})).await;
        let tables = ["workflows", "workflow_runs", "scheduled_workflow_fires"];
        let mut before = Vec::new();
        for table in tables {
            before.push(state(&db, c, table).await);
        }
        resume_tx.send(()).unwrap();
        let error = task.await.unwrap().unwrap_err();
        assert!(
            error.to_string().contains("new dependent records"),
            "{error}"
        );
        for (i, table) in tables.iter().enumerate() {
            assert_eq!(state(&db, c, table).await, before[i]);
        }
        assert_eq!(
            db.user_deletion_store()
                .run_message_batches(
                    &options(c, &alice),
                    ExecutionMode::Delete,
                    None,
                    &mut Vec::new()
                )
                .await
                .unwrap()
                .items,
            0
        );
    }
}
