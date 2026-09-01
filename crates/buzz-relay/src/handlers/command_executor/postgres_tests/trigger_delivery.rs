// Keep out-of-line PostgreSQL cases structurally discoverable by the CI guard.
mod postgres_tests {
    use super::super::*;
    use std::collections::{HashMap, HashSet};

    fn response_run_id(result: &IngestResult) -> Uuid {
        assert!(result.accepted);
        let response: serde_json::Value = serde_json::from_str(
            result
                .message
                .strip_prefix("response:")
                .expect("new run response"),
        )
        .expect("response JSON");
        Uuid::parse_str(response["run_id"].as_str().expect("run ID")).expect("run UUID")
    }

    async fn assert_persisted_trigger_runs(
        state: &AppState,
        tenant: &TenantContext,
        workflow_id: Uuid,
        revision: &Event,
        triggers: &[Event],
        run_ids: &HashSet<Uuid>,
    ) {
        let runs = state
            .db
            .list_workflow_runs(tenant.community(), workflow_id, 100)
            .await
            .expect("persisted runs");
        assert_eq!(
            runs.len(),
            triggers.len(),
            "exactly one run per signed operation"
        );
        assert_eq!(
            run_ids.len(),
            triggers.len(),
            "distinct acknowledged run IDs"
        );
        assert_eq!(
            runs.iter().map(|run| run.id).collect::<HashSet<_>>(),
            *run_ids
        );
        let trigger_ids = triggers
            .iter()
            .map(|trigger| trigger.id.as_bytes().to_vec())
            .collect::<HashSet<_>>();
        assert_eq!(
            trigger_ids.len(),
            triggers.len(),
            "fixture requests must be distinct"
        );
        assert_eq!(
            runs.iter()
                .map(|run| run.trigger_event_id.clone().expect("trigger association"))
                .collect::<HashSet<_>>(),
            trigger_ids
        );
        for run in runs {
            assert_eq!(
                run.definition_event_id.as_deref(),
                Some(revision.id.as_bytes().as_slice())
            );
        }
        for trigger in triggers {
            assert_eq!(
                state
                    .db
                    .get_workflow_run_id_by_trigger(
                        tenant.community(),
                        Uuid::new_v4(),
                        trigger.id.as_bytes(),
                    )
                    .await
                    .expect("workflow-scoped lookup"),
                None
            );
            let stored = state
                .db
                .get_event_by_id(tenant.community(), trigger.id.as_bytes())
                .await
                .expect("trigger remains stored");
            assert!(
                stored.is_some(),
                "later triggers must not replace earlier operations"
            );
        }
    }

    async fn deliver_and_replay(same_second: bool) {
        let (state, tenant, human, _agent, workflow_id, revision) =
            manual_trigger_test_context().await;
        let now = Timestamp::now().as_secs();
        let mut triggers = vec![
            workflow_trigger_event_at(&human, workflow_id, &revision.id.to_hex(), now, "first"),
            workflow_trigger_event_at(
                &human,
                workflow_id,
                &revision.id.to_hex(),
                if same_second { now } else { now - 1 },
                "second",
            ),
        ];
        if same_second {
            // A lower ID wins a NIP-33 timestamp tie. Deliver it first so accidental
            // coordinate replacement would incorrectly suppress the second request.
            triggers.sort_by_key(|event| event.id);
        }
        assert_ne!(triggers[0].id, triggers[1].id);
        assert_eq!(
            triggers[0].created_at == triggers[1].created_at,
            same_second
        );
        let mut run_ids = HashSet::new();
        let mut responses = HashMap::new();
        for trigger in &triggers {
            let result = handle_workflow_trigger(&tenant, &state, trigger, &http_auth(&human))
                .await
                .expect("distinct trigger accepted");
            assert!(run_ids.insert(response_run_id(&result)));
            responses.insert(trigger.id, result.message);
        }
        // Replay both, including the earlier operation after another has committed.
        for trigger in &triggers {
            let replay = handle_workflow_trigger(&tenant, &state, trigger, &http_auth(&human))
                .await
                .expect("exact replay accepted");
            assert!(replay.accepted);
            assert_eq!(
                replay.message, responses[&trigger.id],
                "retry recovers the original result"
            );
        }
        assert_persisted_trigger_runs(&state, &tenant, workflow_id, &revision, &triggers, &run_ids)
            .await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn manual_triggers_delivered_newer_then_older_create_distinct_runs() {
        deliver_and_replay(false).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn manual_triggers_in_one_second_create_distinct_runs() {
        deliver_and_replay(true).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_human_owner_manual_triggers_do_not_starve_one_connection_pool() {
        let (state, tenant, human, _agent, workflow_id, revision) =
            manual_trigger_test_context().await;
        let now = Timestamp::now().as_secs();
        let triggers = (0..8)
            .map(|i| {
                workflow_trigger_event_at(
                    &human,
                    workflow_id,
                    &revision.id.to_hex(),
                    now,
                    &i.to_string(),
                )
            })
            .collect::<Vec<_>>();
        let mut deliveries = triggers.clone();
        // Concurrent exact retries must not create additional runs either.
        deliveries.extend_from_slice(&triggers[..2]);
        let results = tokio::time::timeout(std::time::Duration::from_secs(8), async {
            let mut tasks = tokio::task::JoinSet::new();
            for trigger in deliveries {
                let state = Arc::clone(&state);
                let tenant = tenant.clone();
                let auth = http_auth(&human);
                tasks.spawn(async move {
                    handle_workflow_trigger(&tenant, &state, &trigger, &auth).await
                });
            }
            let mut results = Vec::new();
            while let Some(result) = tasks.join_next().await {
                results.push(
                    result
                        .expect("trigger task must not panic")
                        .expect("trigger succeeds"),
                );
            }
            results
        })
        .await
        .expect("concurrent triggers must drain rather than pool-starve");
        assert_eq!(results.len(), 10);
        assert!(results.iter().all(|result| result.accepted));
        let mut acknowledged = HashMap::new();
        for result in &results {
            let run_id = response_run_id(result);
            if let Some(previous) = acknowledged.insert(result.event_id.clone(), run_id) {
                assert_eq!(
                    previous, run_id,
                    "concurrent retry returns the original run"
                );
            }
        }
        assert_eq!(acknowledged.len(), 8);
        let run_ids = acknowledged.into_values().collect::<HashSet<_>>();
        assert_persisted_trigger_runs(&state, &tenant, workflow_id, &revision, &triggers, &run_ids)
            .await;
    }
}
