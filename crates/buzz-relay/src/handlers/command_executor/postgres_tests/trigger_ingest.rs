// Full WS/HTTP shared ingest, real PostgreSQL event/run transaction and executor.
// Only wall-clock time and the external action sink are controlled by the test.
mod postgres_tests {
    use super::super::*;
    use crate::handlers::ingest::{ingest_event, ingest_event_at};
    use nostr::JsonUtil;
    use std::time::Duration;

    const YAML: &str = "name: manual-trigger-pool\ntrigger:\n  on: message_posted\nsteps:\n  - id: send\n    action: send_message\n    text: done\n";

    fn run_id(response: &IngestResult) -> Uuid {
        let value: serde_json::Value =
            serde_json::from_str(response.message.strip_prefix("response:").unwrap()).unwrap();
        Uuid::parse_str(value["run_id"].as_str().unwrap()).unwrap()
    }

    async fn settled(state: &AppState, tenant: &TenantContext, id: Uuid) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let run = state
                    .db
                    .get_workflow_run(tenant.community(), id)
                    .await
                    .unwrap();
                if run.status == RunStatus::Completed {
                    break;
                }
                assert_ne!(run.status, RunStatus::Failed, "{:?}", run.error_message);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("executor completed");
    }

    async fn absent(state: &AppState, tenant: &TenantContext, workflow: Uuid, event: &Event) {
        assert!(state
            .db
            .get_event_by_id_including_deleted(tenant.community(), event.id.as_bytes())
            .await
            .unwrap()
            .is_none());
        assert!(state
            .db
            .get_workflow_run_id_by_trigger(tenant.community(), workflow, event.id.as_bytes())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn expired_manual_ingest_recovers_committed_exact_bytes_without_dispatch() {
        let (state, tenant, human, _, workflow, revision) =
            manual_trigger_test_context_with_yaml(YAML).await;
        let sink = Arc::new(RecordingActionSink::default());
        state.workflow_engine.set_action_sink(sink.clone());
        let event = workflow_trigger_event(&human, workflow, &revision);
        let bytes = event.as_json();
        let trace_file = tempfile::NamedTempFile::new().unwrap();
        let mut state = Arc::try_unwrap(state).ok().unwrap();
        state.tracer =
            Arc::new(crate::conformance::JsonlTracer::create(trace_file.path()).unwrap());
        let state = Arc::new(state);
        // The caller discards this response (lost delivery); DB/dispatch are real.
        let first = ingest_event(&state, &tenant, event.clone(), http_auth(&human))
            .await
            .unwrap();
        let id = run_id(&first);
        settled(&state, &tenant, id).await;
        assert_eq!(sink.messages.lock().unwrap().as_slice(), ["done"]);
        let later = event.created_at.as_secs() as i64 + 901;
        // Assert recovery's new early return, not the pre-existing
        // fresh-command trace gap.
        let previous_steps = std::fs::read_to_string(trace_file.path())
            .unwrap()
            .lines()
            .count();
        for _ in 0..2 {
            let retry = ingest_event_at(
                &state,
                &tenant,
                Event::from_json(&bytes).unwrap(),
                http_auth(&human),
                later,
            )
            .await
            .expect("expired committed retry recovers");
            assert_eq!(retry.event_id, first.event_id);
            assert_eq!(retry.message, first.message);
        }
        let trace = std::fs::read_to_string(trace_file.path()).unwrap();
        let steps: Vec<buzz_conformance::TraceStep> = trace
            .lines()
            .skip(previous_steps)
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(steps.len(), 2);
        assert!(
            steps.iter().all(|step| matches!(
                step.action,
                buzz_conformance::TraceAction::WriteDuplicate { .. }
            )),
            "recovery must emit duplicate acknowledgment, not a coverage breach: {trace}"
        );
        // A deliberate fresh intent is a different authentic ID, not equal content.
        let distinct = workflow_trigger_event_at(
            &human,
            workflow,
            &revision.id.to_hex(),
            later as u64,
            "distinct",
        );
        let second = ingest_event_at(&state, &tenant, distinct, http_auth(&human), later)
            .await
            .unwrap();
        assert_ne!(id, run_id(&second));
        settled(&state, &tenant, run_id(&second)).await;
        let runs = state
            .db
            .list_workflow_runs(tenant.community(), workflow, 100)
            .await
            .unwrap();
        assert_eq!(runs.len(), 2);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(sink.messages.lock().unwrap().as_slice(), ["done", "done"]);
        assert_eq!(
            state
                .db
                .get_event_by_id(tenant.community(), event.id.as_bytes())
                .await
                .unwrap()
                .unwrap()
                .event
                .as_json(),
            bytes
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn expired_manual_ingest_rejects_unseen_and_invalid_envelopes() {
        let (state, tenant, human, _, workflow, revision) =
            manual_trigger_test_context_with_yaml(YAML).await;
        let sink = Arc::new(RecordingActionSink::default());
        state.workflow_engine.set_action_sink(sink.clone());
        let event = workflow_trigger_event(&human, workflow, &revision);
        let later = event.created_at.as_secs() as i64 + 901;
        assert!(
            ingest_event_at(&state, &tenant, event.clone(), http_auth(&human), later)
                .await
                .is_err()
        );
        absent(&state, &tenant, workflow, &event).await;
        // Far-future unseen events and other kinds retain freshness rejection too.
        assert!(ingest_event_at(
            &state,
            &tenant,
            event.clone(),
            http_auth(&human),
            later - 1802
        )
        .await
        .is_err());
        assert!(
            ingest_event_at(&state, &tenant, revision.clone(), http_auth(&human), later)
                .await
                .is_err()
        );
        let first = ingest_event(&state, &tenant, event.clone(), http_auth(&human))
            .await
            .unwrap();
        settled(&state, &tenant, run_id(&first)).await;
        let mut tamper = event.clone();
        tamper.content = "tampered".into();
        assert!(
            ingest_event_at(&state, &tenant, tamper, http_auth(&human), later)
                .await
                .is_err()
        );
        let mut bad_sig = event.clone();
        bad_sig.sig = workflow_trigger_event(&Keys::generate(), workflow, &revision).sig;
        assert!(
            ingest_event_at(&state, &tenant, bad_sig, http_auth(&human), later)
                .await
                .is_err()
        );
        let stranger = Keys::generate();
        assert!(
            ingest_event_at(&state, &tenant, event.clone(), http_auth(&stranger), later)
                .await
                .is_err()
        );
        let unauthorized = workflow_trigger_event(&stranger, workflow, &revision);
        assert!(ingest_event_at(
            &state,
            &tenant,
            unauthorized.clone(),
            http_auth(&stranger),
            later
        )
        .await
        .is_err());
        absent(&state, &tenant, workflow, &unauthorized).await;
        let mut auth = http_auth(&human);
        if let IngestAuth::Http { scopes, .. } = &mut auth {
            scopes.clear();
        }
        assert!(ingest_event_at(&state, &tenant, event.clone(), auth, later)
            .await
            .is_err());
        let wrong_workflow = workflow_trigger_event(&human, Uuid::new_v4(), &revision);
        assert!(ingest_event_at(
            &state,
            &tenant,
            wrong_workflow.clone(),
            http_auth(&human),
            later
        )
        .await
        .is_err());
        absent(&state, &tenant, workflow, &wrong_workflow).await;
        let other_host = format!("other-{}.example", Uuid::new_v4());
        let other = state
            .db
            .ensure_configured_community(&other_host)
            .await
            .unwrap()
            .id;
        let other_tenant = TenantContext::resolved(other, other_host);
        assert!(ingest_event_at(
            &state,
            &other_tenant,
            event.clone(),
            http_auth(&human),
            later
        )
        .await
        .is_err());
        absent(&state, &other_tenant, workflow, &event).await;
        assert_eq!(
            state
                .db
                .list_workflow_runs(tenant.community(), workflow, 100)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(sink.messages.lock().unwrap().as_slice(), ["done"]);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn expired_manual_ingest_requires_current_authority() {
        for change in ["deleted", "replaced", "nonmember", "disabled"] {
            let (state, tenant, human, agent, workflow, revision) =
                manual_trigger_test_context_with_yaml(YAML).await;
            let sink = Arc::new(RecordingActionSink::default());
            state.workflow_engine.set_action_sink(sink.clone());
            let event = workflow_trigger_event(&human, workflow, &revision);
            let first = ingest_event(&state, &tenant, event.clone(), http_auth(&human))
                .await
                .unwrap();
            settled(&state, &tenant, run_id(&first)).await;
            let community = tenant.community();
            let channel = Uuid::parse_str(exact_tag_value(&revision, "h").unwrap()).unwrap();
            match change {
                "deleted" => {
                    state
                        .db
                        .soft_delete_event(community, revision.id.as_bytes())
                        .await
                        .unwrap();
                }
                "replaced" => {
                    let replacement = EventBuilder::new(
                        Kind::Custom(KIND_WORKFLOW_DEF as u16),
                        YAML.replace("done", "changed"),
                    )
                    .tags(revision.tags.to_vec())
                    .custom_created_at(Timestamp::from(revision.created_at.as_secs() + 1))
                    .sign_with_keys(&agent)
                    .unwrap();
                    let (_, json) =
                        buzz_workflow::WorkflowEngine::parse_yaml(&replacement.content).unwrap();
                    let mut tx = state.db.begin_event_write_transaction().await.unwrap();
                    state
                        .db
                        .replace_parameterized_event_in_transaction(
                            &mut tx,
                            community,
                            &replacement,
                            &workflow.to_string(),
                            Some(channel),
                            buzz_db::replaceable::ParameterizedReplacePrecondition::Unconditional,
                        )
                        .await
                        .unwrap();
                    state
                        .db
                        .upsert_workflow(
                            &mut tx,
                            community,
                            workflow,
                            Some(channel),
                            agent.public_key().as_bytes(),
                            "manual-trigger-pool",
                            &json,
                            &compute_definition_hash(&json),
                            replacement.id.as_bytes(),
                        )
                        .await
                        .unwrap();
                    tx.commit().await.unwrap();
                }
                "nonmember" => {
                    // Preserve a channel owner so removing the workflow principal
                    // exercises revocation rather than the last-owner safety gate.
                    state
                        .db
                        .add_member(
                            community,
                            channel,
                            human.public_key().as_bytes(),
                            buzz_core::channel::MemberRole::Owner,
                            Some(agent.public_key().as_bytes()),
                        )
                        .await
                        .unwrap();
                    state
                        .db
                        .remove_member(
                            community,
                            channel,
                            agent.public_key().as_bytes(),
                            agent.public_key().as_bytes(),
                        )
                        .await
                        .unwrap();
                }
                "disabled" => {
                    state
                        .db
                        .set_workflow_enabled(community, workflow, false)
                        .await
                        .unwrap();
                }
                _ => unreachable!(),
            }
            assert!(
                ingest_event_at(
                    &state,
                    &tenant,
                    event.clone(),
                    http_auth(&human),
                    event.created_at.as_secs() as i64 + 901
                )
                .await
                .is_err(),
                "{change}"
            );
            assert_eq!(
                state
                    .db
                    .list_workflow_runs(community, workflow, 100)
                    .await
                    .unwrap()
                    .len(),
                1
            );
            assert_eq!(sink.messages.lock().unwrap().as_slice(), ["done"]);
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn expired_manual_ingest_never_repairs_event_only_or_retention_gaps_by_execution() {
        let (state, tenant, human, _, workflow, revision) =
            manual_trigger_test_context_with_yaml(YAML).await;
        let sink = Arc::new(RecordingActionSink::default());
        state.workflow_engine.set_action_sink(sink.clone());
        let event = workflow_trigger_event(&human, workflow, &revision);
        let channel = Uuid::parse_str(exact_tag_value(&revision, "h").unwrap()).unwrap();
        // Legacy/partial data: normal manual admission commits event+run atomically.
        let PersistResult::Inserted(tx) =
            persist_command_event(&state.db, &tenant, &event, Some(channel))
                .await
                .unwrap()
        else {
            panic!("new event")
        };
        tx.commit().await.unwrap();
        let later = event.created_at.as_secs() as i64 + 901;
        assert!(
            ingest_event_at(&state, &tenant, event.clone(), http_auth(&human), later)
                .await
                .is_err()
        );
        assert!(state
            .db
            .list_workflow_runs(tenant.community(), workflow, 100)
            .await
            .unwrap()
            .is_empty());
        state
            .db
            .soft_delete_event(tenant.community(), event.id.as_bytes())
            .await
            .unwrap();
        assert!(
            ingest_event_at(&state, &tenant, event.clone(), http_auth(&human), later)
                .await
                .is_err()
        );
        assert!(sink.messages.lock().unwrap().is_empty());
    }
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn expired_manual_ingest_acknowledges_pending_run_without_rescheduling() {
        let (state, tenant, human, _, workflow, revision) =
            manual_trigger_test_context_with_yaml(YAML).await;
        let sink = Arc::new(RecordingActionSink::default());
        state.workflow_engine.set_action_sink(sink.clone());
        let event = workflow_trigger_event(&human, workflow, &revision);
        let channel = Uuid::parse_str(exact_tag_value(&revision, "h").unwrap()).unwrap();
        let community = tenant.community();
        // Reproduce the durable state at the real commit -> spawn crash window.
        // Run creation is in the event transaction, not deferred to the executor.
        let PersistResult::Inserted(mut tx) =
            persist_command_event(&state.db, &tenant, &event, Some(channel))
                .await
                .unwrap()
        else {
            panic!("new event")
        };
        let id = state
            .db
            .create_workflow_run_in_transaction(
                &mut tx,
                community,
                workflow,
                revision.id.as_bytes(),
                Some(event.id.as_bytes()),
                None,
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
        for status in [
            RunStatus::Pending,
            RunStatus::Running,
            RunStatus::WaitingApproval,
            RunStatus::Failed,
        ] {
            state
                .db
                .update_workflow_run(
                    community,
                    id,
                    status.clone(),
                    0,
                    &serde_json::json!([]),
                    None,
                )
                .await
                .unwrap();
            let response = ingest_event_at(
                &state,
                &tenant,
                event.clone(),
                http_auth(&human),
                event.created_at.as_secs() as i64 + 901,
            )
            .await
            .unwrap();
            assert_eq!(run_id(&response), id);
            tokio::time::sleep(Duration::from_millis(25)).await;
            assert_eq!(
                state
                    .db
                    .get_workflow_run(community, id)
                    .await
                    .unwrap()
                    .status,
                status
            );
            assert!(
                sink.messages.lock().unwrap().is_empty(),
                "acknowledgment is not a retry of execution"
            );
        }
        // Retention can also remove the event while the run remains. Do not
        // mistake the surviving run reference for fresh execution authorization.
        state
            .db
            .soft_delete_event(community, event.id.as_bytes())
            .await
            .unwrap();
        assert!(ingest_event_at(
            &state,
            &tenant,
            event.clone(),
            http_auth(&human),
            event.created_at.as_secs() as i64 + 901
        )
        .await
        .is_err());
        assert_eq!(
            state
                .db
                .list_workflow_runs(community, workflow, 100)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(sink.messages.lock().unwrap().is_empty());
    }
}
