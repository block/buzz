//! Redeploy orchestration races: per-agent serialization + completion binding.
//!
//! These drive the real decision logic (`provider_access::run_deploy` /
//! `bind_completion`) through an in-memory store and a fake provider gated on
//! oneshot channels, so mid-flight edits and deletions are deterministic.

use super::provider_access;
use super::tests::bare_agent_record;
use crate::managed_agents::ManagedAgentRecord;

fn provider_backed_record(pubkey: &str, model: &str, updated_at: &str) -> ManagedAgentRecord {
    use crate::managed_agents::BackendKind;

    let mut record = bare_agent_record(None, Some(model), None);
    record.pubkey = pubkey.to_string();
    record.updated_at = updated_at.to_string();
    record.backend = BackendKind::Provider {
        id: "provider".to_string(),
        config: serde_json::json!({"region": "test"}),
    };
    record.backend_agent_id = Some("existing".to_string());
    record
}

/// In-memory [`provider_access::DeployEffects`]: the store is a shared Vec,
/// the "provider" records the model of every payload it receives and can be
/// gated (blocked until released) to hold a redeploy in flight. `saves`
/// counts the completions that asked for a store write, so tests can assert
/// that a deletion or backend switch produced no write at all.
struct FakeDeployEffects {
    store: std::sync::Arc<std::sync::Mutex<Vec<ManagedAgentRecord>>>,
    sent: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    saves: std::sync::Arc<std::sync::Mutex<u32>>,
    started: Option<tokio::sync::oneshot::Sender<()>>,
    release: Option<tokio::sync::oneshot::Receiver<()>>,
    result: Result<String, String>,
}

impl provider_access::DeployEffects for FakeDeployEffects {
    type Success = ManagedAgentRecord;

    fn snapshot(&mut self, pubkey: &str) -> Result<provider_access::DeploySnapshot, String> {
        let store = self.store.lock().unwrap();
        let record = store
            .iter()
            .find(|record| record.pubkey == pubkey)
            .ok_or_else(|| format!("agent {pubkey} not found"))?
            .clone();
        let captured = provider_access::redeploy_provider_target(pubkey, &record)?;
        Ok(provider_access::DeploySnapshot { record, captured })
    }

    async fn deploy(
        &mut self,
        snapshot: &provider_access::DeploySnapshot,
    ) -> Result<String, String> {
        if let Some(started) = self.started.take() {
            let _ = started.send(());
        }
        if let Some(release) = self.release.take() {
            let _ = release.await;
        }
        self.sent
            .lock()
            .unwrap()
            .push(snapshot.record.model.clone().unwrap_or_default());
        self.result.clone()
    }

    fn with_store<R>(
        &mut self,
        apply: &mut dyn FnMut(&mut Vec<ManagedAgentRecord>) -> (bool, R),
    ) -> Result<R, String> {
        let mut store = self.store.lock().unwrap();
        let (save, result) = apply(&mut store);
        if save {
            *self.saves.lock().unwrap() += 1;
        }
        Ok(result)
    }

    fn success(&mut self, record: &ManagedAgentRecord) -> Result<ManagedAgentRecord, String> {
        Ok(record.clone())
    }
}

/// The reviewer's race: redeploy A blocks in the provider with config v1, an
/// owner edit bumps the store to v2, redeploy B is issued, then A completes.
/// The per-agent lock queues B behind A, so the provider must see v1 then v2
/// (newest payload wins remotely), Desktop state must end on B, and A must not
/// report the edited record as deployed.
#[tokio::test]
async fn redeploy_race_late_completion_does_not_claim_newer_config() {
    use std::sync::{Arc, Mutex};

    let pubkey = "race-agent";
    let store = Arc::new(Mutex::new(vec![provider_backed_record(
        pubkey, "model-v1", "t1",
    )]));
    let sent = Arc::new(Mutex::new(Vec::new()));
    let saves = Arc::new(Mutex::new(0));

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();

    let mut effects_a = FakeDeployEffects {
        store: store.clone(),
        sent: sent.clone(),
        saves: saves.clone(),
        started: Some(started_tx),
        release: Some(release_rx),
        result: Ok("deploy-a".to_string()),
    };
    let task_a =
        tokio::spawn(async move { provider_access::run_deploy(pubkey, &mut effects_a).await });
    started_rx.await.expect("redeploy A entered the provider");

    // Owner edit while A is blocked in the provider: config generation bumps.
    {
        let mut records = store.lock().unwrap();
        let record = records
            .iter_mut()
            .find(|record| record.pubkey == pubkey)
            .unwrap();
        record.model = Some("model-v2".to_string());
        record.updated_at = "t2".to_string();
    }

    let mut effects_b = FakeDeployEffects {
        store: store.clone(),
        sent: sent.clone(),
        saves: saves.clone(),
        started: None,
        release: None,
        result: Ok("deploy-b".to_string()),
    };
    let task_b =
        tokio::spawn(async move { provider_access::run_deploy(pubkey, &mut effects_b).await });
    // Let B reach the per-agent lock so it is genuinely queued behind A.
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }

    release_tx.send(()).expect("release redeploy A");
    let result_a = task_a.await.expect("join A");
    let result_b = task_b.await.expect("join B");

    // A deployed the old config; it must not claim the current record was applied.
    let error_a = result_a.expect_err("A must not report success-as-current");
    assert!(
        error_a.contains("running the previous configuration"),
        "unexpected A error: {error_a}"
    );

    // B applied the latest config as current.
    let applied_b = result_b.expect("B applies the latest config");
    assert_eq!(applied_b.model.as_deref(), Some("model-v2"));
    assert_eq!(applied_b.backend_agent_id.as_deref(), Some("deploy-b"));

    // The provider saw v1 then v2 — the newest payload wins remotely.
    assert_eq!(
        *sent.lock().unwrap(),
        vec!["model-v1".to_string(), "model-v2".to_string()]
    );

    // Desktop state ends on B: v2 config with B's deployment id, no error.
    let records = store.lock().unwrap();
    let record = records
        .iter()
        .find(|record| record.pubkey == pubkey)
        .unwrap();
    assert_eq!(record.model.as_deref(), Some("model-v2"));
    assert_eq!(record.backend_agent_id.as_deref(), Some("deploy-b"));
    assert!(record.last_started_at.is_some());
    assert_eq!(record.last_error, None);

    // Both completions persisted remote facts: A's stale stamp and B's current.
    assert_eq!(*saves.lock().unwrap(), 2);
}

/// Deletion during the provider call: the completed deploy must not resurrect
/// the record, and the caller is told the remote deployment may still exist.
#[tokio::test]
async fn redeploy_deletion_mid_flight_reports_and_does_not_resurrect() {
    use std::sync::{Arc, Mutex};

    let pubkey = "deleted-agent";
    let store = Arc::new(Mutex::new(vec![provider_backed_record(
        pubkey, "model-v1", "t1",
    )]));
    let sent = Arc::new(Mutex::new(Vec::new()));
    let saves = Arc::new(Mutex::new(0));

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();

    let mut effects = FakeDeployEffects {
        store: store.clone(),
        sent: sent.clone(),
        saves: saves.clone(),
        started: Some(started_tx),
        release: Some(release_rx),
        result: Ok("deploy-a".to_string()),
    };
    let task = tokio::spawn(async move { provider_access::run_deploy(pubkey, &mut effects).await });
    started_rx.await.expect("redeploy entered the provider");

    // Owner deletes the agent while the provider call is in flight.
    store.lock().unwrap().clear();

    release_tx.send(()).expect("release redeploy");
    let error = task
        .await
        .expect("join redeploy")
        .expect_err("deleted agent must not report success");
    assert!(
        error.contains("deleted while the deploy") && error.contains("may still be running"),
        "unexpected error: {error}"
    );
    assert!(
        store.lock().unwrap().is_empty(),
        "completion resurrected a deleted record"
    );
    assert_eq!(*saves.lock().unwrap(), 0, "deletion must not cause a write");
}

/// Backend switch during the provider call (Provider → Local here; a
/// provider-id change is covered by the `bind_completion` unit test): the
/// completed deploy belongs to the old backend, so nothing may be stamped
/// onto the re-homed record and the caller is told the remote deployment may
/// still exist.
#[tokio::test]
async fn redeploy_backend_switch_mid_flight_reports_and_stamps_nothing() {
    use std::sync::{Arc, Mutex};

    use crate::managed_agents::BackendKind;

    let pubkey = "switched-agent";
    let store = Arc::new(Mutex::new(vec![provider_backed_record(
        pubkey, "model-v1", "t1",
    )]));
    let sent = Arc::new(Mutex::new(Vec::new()));
    let saves = Arc::new(Mutex::new(0));

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();

    let mut effects = FakeDeployEffects {
        store: store.clone(),
        sent: sent.clone(),
        saves: saves.clone(),
        started: Some(started_tx),
        release: Some(release_rx),
        result: Ok("deploy-a".to_string()),
    };
    let task = tokio::spawn(async move { provider_access::run_deploy(pubkey, &mut effects).await });
    started_rx.await.expect("redeploy entered the provider");

    // Owner switches the agent to the local backend while the call is in flight.
    {
        let mut records = store.lock().unwrap();
        let record = records
            .iter_mut()
            .find(|record| record.pubkey == pubkey)
            .unwrap();
        record.backend = BackendKind::Local;
        record.backend_agent_id = None;
        record.updated_at = "t2".to_string();
    }

    release_tx.send(()).expect("release redeploy");
    let error = task
        .await
        .expect("join redeploy")
        .expect_err("switched-backend agent must not report success");
    assert!(
        error.contains("backend changed") && error.contains("may still be running"),
        "unexpected error: {error}"
    );

    // The re-homed record is untouched by the completion, and nothing was
    // written to the store at all.
    let records = store.lock().unwrap();
    let record = records
        .iter()
        .find(|record| record.pubkey == pubkey)
        .unwrap();
    assert_eq!(record.backend, BackendKind::Local);
    assert_eq!(record.backend_agent_id, None);
    assert_eq!(record.updated_at, "t2");
    assert!(record.last_started_at.is_none());
    assert_eq!(
        *saves.lock().unwrap(),
        0,
        "backend switch must not cause a write"
    );
}

/// Direct unit coverage of the completion-binding branches.
#[test]
fn redeploy_completion_binding_branches() {
    use provider_access::{bind_completion, CompletionOutcome};

    // Unchanged: success persisted as current, last_error cleared.
    let mut records = vec![provider_backed_record("agent", "model-v1", "t1")];
    records[0].last_error = Some("old failure".to_string());
    let (save, outcome) = bind_completion(
        &mut records,
        "agent",
        "provider",
        "t1",
        &Ok("deploy-1".to_string()),
    );
    assert!(save);
    assert!(matches!(outcome, CompletionOutcome::AppliedCurrent(_)));
    assert_eq!(records[0].backend_agent_id.as_deref(), Some("deploy-1"));
    assert_eq!(records[0].last_error, None);
    assert!(records[0].last_started_at.is_some());

    // Changed mid-flight: remote facts persisted, but nothing implies currency —
    // last_error stays.
    let mut records = vec![provider_backed_record("agent", "model-v2", "t2")];
    records[0].last_error = Some("earlier failure".to_string());
    let (save, outcome) = bind_completion(
        &mut records,
        "agent",
        "provider",
        "t1",
        &Ok("deploy-2".to_string()),
    );
    assert!(save);
    assert!(matches!(outcome, CompletionOutcome::AppliedStale));
    assert_eq!(records[0].backend_agent_id.as_deref(), Some("deploy-2"));
    assert_eq!(records[0].last_error.as_deref(), Some("earlier failure"));

    // Deleted: nothing to write, nothing resurrected.
    let mut records: Vec<ManagedAgentRecord> = vec![];
    let (save, outcome) = bind_completion(
        &mut records,
        "agent",
        "provider",
        "t1",
        &Ok("deploy-3".to_string()),
    );
    assert!(!save);
    assert!(matches!(outcome, CompletionOutcome::Deleted));
    assert!(records.is_empty());

    // Moved to a different provider mid-flight: the deploy landed on the old
    // provider, so its id must not be stamped onto the re-homed record.
    let mut records = vec![provider_backed_record("agent", "model-v1", "t2")];
    records[0].backend = crate::managed_agents::BackendKind::Provider {
        id: "other-provider".to_string(),
        config: serde_json::json!({}),
    };
    let (save, outcome) = bind_completion(
        &mut records,
        "agent",
        "provider",
        "t1",
        &Ok("deploy-4".to_string()),
    );
    assert!(!save);
    assert!(matches!(outcome, CompletionOutcome::BackendChanged));
    assert_eq!(records[0].backend_agent_id.as_deref(), Some("existing"));
    assert_eq!(records[0].updated_at, "t2");

    // Provider failure with a surviving record: failure stamped, id untouched.
    let mut records = vec![provider_backed_record("agent", "model-v1", "t1")];
    let (save, outcome) = bind_completion(
        &mut records,
        "agent",
        "provider",
        "t1",
        &Err("boom".to_string()),
    );
    assert!(save);
    assert!(matches!(outcome, CompletionOutcome::Failed(_)));
    assert_eq!(records[0].last_error.as_deref(), Some("boom"));
    assert_eq!(records[0].backend_agent_id.as_deref(), Some("existing"));
}

/// Direct unit coverage of the outcome → command-result fold shared by the
/// redeploy command and `deploy_to_provider` (create/start/reconcile). This is
/// the testable seam for the deploy paths, which otherwise need a real Tauri
/// app handle: `deploy_to_provider` is exactly `bind_completion` (covered
/// above) + this fold.
#[test]
fn deploy_completion_result_mirrors_redeploy_semantics() {
    use provider_access::{completion_result, CompletionOutcome};

    // Applied as current: the freshly persisted record comes back.
    let record = provider_backed_record("agent", "model-v1", "t1");
    let applied = completion_result(
        CompletionOutcome::AppliedCurrent(Box::new(record)),
        Ok("deploy-1".to_string()),
    )
    .expect("applied current is success");
    assert_eq!(applied.pubkey, "agent");

    // Stale: the deploy happened, but success must not claim currency.
    let error = completion_result(CompletionOutcome::AppliedStale, Ok("deploy-2".to_string()))
        .expect_err("stale completion is not success");
    assert!(
        error.contains("running the previous configuration"),
        "unexpected error: {error}"
    );

    // Backend switched away mid-flight: honest orphan warning.
    let error = completion_result(
        CompletionOutcome::BackendChanged,
        Ok("deploy-3".to_string()),
    )
    .expect_err("backend change is not success");
    assert!(
        error.contains("backend changed") && error.contains("may still be running"),
        "unexpected error: {error}"
    );

    // Deleted mid-flight: orphan warning on a successful deploy, the provider's
    // own error on a failed one.
    let error = completion_result(CompletionOutcome::Deleted, Ok("deploy-4".to_string()))
        .expect_err("deleted agent is not success");
    assert!(
        error.contains("deleted while the deploy") && error.contains("may still be running"),
        "unexpected error: {error}"
    );
    let error = completion_result(CompletionOutcome::Deleted, Err("boom".to_string()))
        .expect_err("deleted agent is not success");
    assert_eq!(error, "boom");

    // Provider failure: surfaced verbatim.
    let error = completion_result(
        CompletionOutcome::Failed("kaput".to_string()),
        Err("kaput".to_string()),
    )
    .expect_err("failed deploy is not success");
    assert_eq!(error, "kaput");
}
