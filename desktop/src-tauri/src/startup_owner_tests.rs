use super::*;

fn owner_identity(
    owner: &str,
    model: &str,
    snapshot: &str,
    config: &str,
    capacity: u8,
) -> RuntimeConfigIdentity {
    RuntimeConfigIdentity::new_for_test(owner, model, snapshot, config, capacity, "policy-v1")
}

#[tokio::test]
async fn runtime_views_and_cancellation_never_signal_another_owner_run() {
    let make = |owner: &str, generation| {
        let config = owner_identity(owner, "qwen", "snapshot-a", "apple-a", 1);
        let scheduler = LocalModelScheduler::new(1).expect("scheduler");
        Arc::new(InstalledCommandBriefRuntime {
            owner_pubkey: owner.to_string(),
            config,
            generation,
            orchestrator: CommandBriefOrchestrator::new(
                scheduler,
                Arc::new(UnusedProvider),
                Arc::new(UnusedProvider),
                Arc::new(UnusedProvider),
            ),
        })
    };
    let request = || {
        CommandBriefRequest::new("daily-command-brief", "prepare", "2026-07-25T06:00:00Z")
            .expect("request")
    };
    let owner_a = "owner-a";
    let owner_b = "owner-b";
    let runtime_a = make(owner_a, 1);
    runtime_a
        .orchestrator
        .start_exact("owner-a-run", request())
        .expect("queued owner A run");
    let mut runtimes = CommandBriefRuntimeSet::default();
    runtimes.install(Arc::clone(&runtime_a));

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if runtimes
                .status(owner_a, "owner-a-run")
                .is_some_and(|status| {
                    status.state() == crate::command_brief::types::BriefRunState::CollectingSources
                })
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owner A active");

    assert!(runtimes.status(owner_a, "owner-a-run").is_some());
    assert!(runtimes.status(owner_b, "owner-a-run").is_none());
    assert!(runtimes.latest_status_and_history(owner_b).is_none());
    assert!(runtimes
        .history_after(owner_b, "owner-a-run", None)
        .is_empty());
    assert!(!runtimes.cancel(owner_b, "owner-a-run"));
    assert!(
        runtimes.status(owner_a, "owner-a-run").is_some(),
        "a denied owner-B cancellation must not affect owner A"
    );

    assert!(runtimes.cancel(owner_a, "owner-a-run"));
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let terminal = runtimes
                .status(owner_a, "owner-a-run")
                .is_some_and(|status| {
                    matches!(
                        status.state(),
                        crate::command_brief::types::BriefRunState::Cancelled
                            | crate::command_brief::types::BriefRunState::Failed
                    )
                });
            if terminal {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owner A terminal");

    assert!(runtimes.status(owner_b, "owner-a-run").is_none());
    assert!(runtimes.latest_status_and_history(owner_b).is_none());
    assert!(runtimes
        .history_after(owner_b, "owner-a-run", None)
        .is_empty());
    assert!(!runtimes.cancel(owner_b, "owner-a-run"));

    let all = runtimes.history_after(owner_a, "owner-a-run", None);
    assert_eq!(
        all.iter()
            .map(|status| status.sequence())
            .collect::<Vec<_>>(),
        vec![0, 1, 2],
        "fast queued, active, and terminal transitions must remain ordered"
    );
    assert_eq!(
        runtimes
            .history_after(owner_a, "owner-a-run", Some(0))
            .iter()
            .map(|status| status.sequence())
            .collect::<Vec<_>>(),
        vec![1, 2],
        "a cursor must return every unseen transition exactly once"
    );
}
