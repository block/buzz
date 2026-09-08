//! Practical same-host acceptance: two simultaneously stored named choices,
//! signed request bytes and native receiver/shared spawn, not mocked IPC.
//! Provider readiness and ACP children remain the existing isolated fixtures.
use super::*;

#[tokio::test]
async fn two_named_choices_bind_explicit_start_and_restart_not_next_selection() {
    let _guard = agents::lock_path_mutex_async().await;
    let mut fixture = Fixture::new();
    let owner = fixture.owner.public_key().to_hex();
    let identity = fixture.record.pubkey.clone();
    let first = fixture.named.clone();
    let mut second = config(&first.host);
    second.name = "Second".into();
    second.model = "second-model".into();
    second.workspace = first.workspace.clone();
    fixture
        .record
        .runtime_configurations
        .replace(
            &owner,
            COMMUNITY,
            &first.host,
            RuntimeConfigurations {
                selected: Some(second.id.clone()),
                entries: vec![first.clone(), second.clone()],
            },
        )
        .unwrap();
    let choices = fixture.record.runtime_configurations.get(&owner, COMMUNITY);
    second = choices
        .entries
        .iter()
        .find(|entry| entry.id == second.id)
        .unwrap()
        .clone();
    fixture.persist();
    assert_eq!(choices.entries.len(), 2);
    assert_eq!(choices.selected.as_deref(), Some(second.id.as_str()));

    // Explicit first is deliberately NOT the selected next configuration.
    // Round-trip encrypted, signed event bytes without inventing a transport.
    let request = fixture.request(Action::Start, None);
    let wire = serde_json::to_vec(&request).unwrap();
    let received: Event = serde_json::from_slice(&wire).unwrap();
    assert_eq!(received, request);
    let started = fixture
        .receive(received, |model, allow| async move {
            assert!(model.is_none());
            assert!(!allow);
            Ok(())
        })
        .await;
    assert_eq!(started.outcome, Outcome::Running);
    assert_eq!(
        started.observation.unwrap().running_configuration,
        Some(first.reference())
    );
    fixture.launched("fixture-model|fixture-model");
    let original = fixture.running().unwrap();
    println!("ACCEPTANCE: two named choices; next=Second; explicit First started; child confirmed exact First model env");

    // Edit First while it runs: receipt remains its launched revision, not the
    // edited revision and not Second (the selected next launch).
    let mut edited = first.clone();
    edited.model = "edited-first-model".into();
    fixture
        .record
        .runtime_configurations
        .replace(
            &owner,
            COMMUNITY,
            &first.host,
            RuntimeConfigurations {
                selected: Some(second.id.clone()),
                entries: vec![edited, second.clone()],
            },
        )
        .unwrap();
    fixture.persist();
    let status = fixture.action(Action::Status, None).await;
    assert_eq!(status.outcome, Outcome::Running);
    assert_eq!(
        status.observation.unwrap().running_configuration,
        Some(first.reference())
    );
    assert_eq!(fixture.running().unwrap(), original);
    // Start of the exact already-running reference is an idempotent running
    // observation, even after its definition was edited. Eligibility for a NEW
    // launch is separate: the stale reference must fail Preflight.
    assert_eq!(
        fixture.action(Action::Start, None).await.outcome,
        Outcome::Running
    );
    assert_eq!(
        fixture.action(Action::Preflight, None).await.outcome,
        Outcome::Ineligible
    );
    assert_eq!(fixture.running().unwrap(), original);
    println!("ACCEPTANCE: edit changed next-launch revision only; old revision ineligible for new launch; idempotent Start reports actual running PID/receipt unchanged");

    fixture.named = second.clone();
    assert_eq!(
        fixture.action(Action::Start, None).await.outcome,
        Outcome::DifferentConfiguration
    );
    assert_eq!(fixture.running().unwrap(), original);
    let failed = fixture
        .receive(
            fixture.request(Action::Restart, Some(status.id)),
            |_, _| async { Err("isolated readiness refusal".into()) },
        )
        .await;
    assert_eq!(failed.outcome, Outcome::Ineligible);
    assert_eq!(fixture.running().unwrap(), original);
    println!("ACCEPTANCE: Second readiness failure refused before Stop; First still alive");

    let status = fixture.action(Action::Status, None).await;
    let switched = fixture.action(Action::Restart, Some(status.id)).await;
    assert_eq!(switched.outcome, Outcome::Running);
    assert_eq!(
        switched.observation.unwrap().running_configuration,
        Some(second.reference())
    );
    let replacement = fixture.running().unwrap();
    assert_ne!(replacement.0, original.0);
    assert_eq!(replacement.1, Some(second.reference()));
    assert_eq!(fixture.record.pubkey, identity);
    fixture.launched("second-model|second-model");
    assert_eq!(
        std::fs::read_to_string(fixture.temp.path().join("launches"))
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec!["fixture-model|fixture-model", "second-model|second-model"]
    );
    println!("ACCEPTANCE: same-host Restart launched exact Second; new PID; stable identity; exactly two child launches, no edited-model substitution");
    assert_eq!(fixture.stop().await, StopOutcome::Stopped);
    assert!(fixture.running().is_none());
    println!(
        "ACCEPTANCE: ordinary signed Stop confirmed; no child remains; model-ready NOT claimed"
    );
}
