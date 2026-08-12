//! Redeploy orchestration races: per-agent serialization and completion
//! binding.
//!
//! These drive the real decision logic (`provider_access::run_deploy` /
//! `bind_completion`) through an in-memory store and a fake provider gated on
//! oneshot channels, so mid-flight edits and deletions are deterministic.
//!
//! The fake models the two halves of a deploy input the way the real one is
//! shaped: the agent *row* (in the store) and the *world* outside it — the
//! persona, team, global agent config, workspace relay URL and owner pubkey
//! that `build_deploy_payload` re-resolves on every call. Both feed
//! [`fake_payload`], and the digest is computed by the shipped
//! `payload_digest::deploy_input_digest`, so a world-only change is exactly
//! the case the reviewer named: the agent row is untouched and `updated_at`
//! does not move.

use std::sync::{Arc, Mutex};

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

/// The stand-in for `build_deploy_payload`: a projection of the agent row plus
/// the world resolved around it.
fn fake_payload(record: &ManagedAgentRecord, world: &str) -> serde_json::Value {
    serde_json::json!({
        "name": record.name,
        "model": record.model,
        // Everything the payload resolves from outside the agent row.
        "resolved_world": world,
    })
}

/// The world the fake resolves payloads against. Mutating it models a persona
/// edit, a global-config change, a relay override or an owner switch — none of
/// which touch the agent row.
#[derive(Clone)]
struct World(Arc<Mutex<String>>);

impl World {
    fn new(value: &str) -> Self {
        World(Arc::new(Mutex::new(value.to_string())))
    }
    fn set(&self, value: &str) {
        *self.0.lock().unwrap() = value.to_string();
    }
    fn read(&self) -> String {
        self.0.lock().unwrap().clone()
    }
}

/// In-memory [`provider_access::DeployEffects`]: the store is a shared Vec,
/// the "provider" records the model of every payload it receives and can be
/// gated (blocked until released) to hold a redeploy in flight. `saves`
/// counts the completions that asked for a store write, so tests can assert
/// that a deletion or backend switch produced no write at all.
struct FakeDeployEffects {
    store: Arc<Mutex<Vec<ManagedAgentRecord>>>,
    world: World,
    sent: Arc<Mutex<Vec<String>>>,
    saves: Arc<Mutex<u32>>,
    started: Option<tokio::sync::oneshot::Sender<()>>,
    release: Option<tokio::sync::oneshot::Receiver<()>>,
    result: Result<String, String>,
    /// Simulates a payload that can no longer be resolved on completion.
    world_unresolvable: bool,
}

impl FakeDeployEffects {
    fn new(
        store: &Arc<Mutex<Vec<ManagedAgentRecord>>>,
        world: &World,
        result: Result<String, String>,
    ) -> Self {
        FakeDeployEffects {
            store: store.clone(),
            world: world.clone(),
            sent: Arc::new(Mutex::new(Vec::new())),
            saves: Arc::new(Mutex::new(0)),
            started: None,
            release: None,
            result,
            world_unresolvable: false,
        }
    }

    fn sharing(mut self, sent: &Arc<Mutex<Vec<String>>>, saves: &Arc<Mutex<u32>>) -> Self {
        self.sent = sent.clone();
        self.saves = saves.clone();
        self
    }

    fn gated(
        mut self,
        started: tokio::sync::oneshot::Sender<()>,
        release: tokio::sync::oneshot::Receiver<()>,
    ) -> Self {
        self.started = Some(started);
        self.release = Some(release);
        self
    }
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
        let payload = fake_payload(&record, &self.world.read());
        let captured = provider_access::redeploy_provider_target(pubkey, &record, &payload)?;
        Ok(provider_access::DeploySnapshot { captured, payload })
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
        self.sent.lock().unwrap().push(
            snapshot.payload["model"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        );
        self.result.clone()
    }

    fn with_store<R>(
        &mut self,
        apply: &mut provider_access::CompletionApply<'_, R>,
    ) -> Result<R, String> {
        let mut store = self.store.lock().unwrap();
        let world = self.world.clone();
        let unresolvable = self.world_unresolvable;
        let resolve_digest = move |record: &ManagedAgentRecord| -> Result<String, String> {
            if unresolvable {
                return Err("persona is missing".to_string());
            }
            provider_access::payload_digest::deploy_input_digest(
                record,
                &fake_payload(record, &world.read()),
            )
        };
        let (save, result) = apply(&mut store, &resolve_digest);
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
    let pubkey = "race-agent";
    let store = Arc::new(Mutex::new(vec![provider_backed_record(
        pubkey, "model-v1", "t1",
    )]));
    let world = World::new("persona-v1");
    let sent = Arc::new(Mutex::new(Vec::new()));
    let saves = Arc::new(Mutex::new(0));

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();

    let mut effects_a = FakeDeployEffects::new(&store, &world, Ok("deploy-a".to_string()))
        .sharing(&sent, &saves)
        .gated(started_tx, release_rx);
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

    let mut effects_b =
        FakeDeployEffects::new(&store, &world, Ok("deploy-b".to_string())).sharing(&sent, &saves);
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
        error_a.contains("sent the previous settings"),
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

/// The reviewer's second boundary: the agent row is untouched — same
/// `updated_at`, same everything the store holds — but an input the payload
/// resolves *outside* the row (here the persona) changed while the provider
/// call was in flight. The old payload must not be stamped `AppliedCurrent`.
#[tokio::test]
async fn persona_change_mid_deploy_is_not_applied_current() {
    let pubkey = "world-agent";
    let store = Arc::new(Mutex::new(vec![provider_backed_record(
        pubkey, "model-v1", "t1",
    )]));
    let world = World::new("persona-v1");

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();

    let mut effects = FakeDeployEffects::new(&store, &world, Ok("deploy-a".to_string()))
        .gated(started_tx, release_rx);
    let task = tokio::spawn(async move { provider_access::run_deploy(pubkey, &mut effects).await });
    started_rx.await.expect("redeploy entered the provider");

    // The persona (or global config, or relay override, or owner) changes.
    // The agent row is deliberately NOT touched.
    world.set("persona-v2");
    let before = store.lock().unwrap()[0].clone();

    release_tx.send(()).expect("release redeploy");
    let error = task
        .await
        .expect("join redeploy")
        .expect_err("a payload resolved from a changed world is not current");
    assert!(
        error.contains("sent the previous settings"),
        "unexpected error: {error}"
    );

    // The agent row never moved, so an `updated_at`-only binding would have
    // called this current.
    assert_eq!(before.updated_at, "t1");

    // The remote facts are still persisted — the deploy really happened.
    let record = store.lock().unwrap()[0].clone();
    assert_eq!(record.backend_agent_id.as_deref(), Some("deploy-a"));
    assert!(record.last_started_at.is_some());
}

/// The same world change, arriving *before* the deploy rather than during it,
/// must still be reported as current — a digest that never matches would make
/// every deploy look stale and the honest report meaningless.
#[tokio::test]
async fn an_unchanged_world_is_reported_as_current() {
    let pubkey = "steady-agent";
    let store = Arc::new(Mutex::new(vec![provider_backed_record(
        pubkey, "model-v1", "t1",
    )]));
    let world = World::new("persona-v2");

    let mut effects = FakeDeployEffects::new(&store, &world, Ok("deploy-a".to_string()));
    let applied = provider_access::run_deploy(pubkey, &mut effects)
        .await
        .expect("an unchanged deploy input is current");

    assert_eq!(applied.backend_agent_id.as_deref(), Some("deploy-a"));
    assert_eq!(applied.last_error, None);
}

/// The payload cannot be re-resolved on completion (the persona the record
/// points at is gone). Currency can be neither proved nor disproved, so the
/// remote facts are persisted and the answer says exactly that.
#[tokio::test]
async fn an_unresolvable_payload_is_reported_as_unverified() {
    let pubkey = "unresolvable-agent";
    let store = Arc::new(Mutex::new(vec![provider_backed_record(
        pubkey, "model-v1", "t1",
    )]));
    let world = World::new("persona-v1");

    let mut effects = FakeDeployEffects::new(&store, &world, Ok("deploy-a".to_string()));
    effects.world_unresolvable = true;
    let error = provider_access::run_deploy(pubkey, &mut effects)
        .await
        .expect_err("an unverifiable completion must not claim currency");

    assert!(
        error.contains("could not re-read") && error.contains("persona is missing"),
        "unexpected error: {error}"
    );
    let record = store.lock().unwrap()[0].clone();
    assert_eq!(
        record.backend_agent_id.as_deref(),
        Some("deploy-a"),
        "the deploy happened; its remote facts must still be persisted"
    );
}

/// Deletion during the provider call: the completed deploy must not resurrect
/// the record, and the caller is told the remote deployment may still exist.
#[tokio::test]
async fn redeploy_deletion_mid_flight_reports_and_does_not_resurrect() {
    let pubkey = "deleted-agent";
    let store = Arc::new(Mutex::new(vec![provider_backed_record(
        pubkey, "model-v1", "t1",
    )]));
    let world = World::new("persona-v1");
    let sent = Arc::new(Mutex::new(Vec::new()));
    let saves = Arc::new(Mutex::new(0));

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();

    let mut effects = FakeDeployEffects::new(&store, &world, Ok("deploy-a".to_string()))
        .sharing(&sent, &saves)
        .gated(started_tx, release_rx);
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
    use crate::managed_agents::BackendKind;

    let pubkey = "switched-agent";
    let store = Arc::new(Mutex::new(vec![provider_backed_record(
        pubkey, "model-v1", "t1",
    )]));
    let world = World::new("persona-v1");
    let sent = Arc::new(Mutex::new(Vec::new()));
    let saves = Arc::new(Mutex::new(0));

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();

    let mut effects = FakeDeployEffects::new(&store, &world, Ok("deploy-a".to_string()))
        .sharing(&sent, &saves)
        .gated(started_tx, release_rx);
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

/// A provider failure whose record moved on: the failure belongs to the
/// configuration that was sent, not to the one the owner has since saved, so
/// `last_error` must not be stamped on it. The owner still sees the error.
#[tokio::test]
async fn provider_failure_does_not_stamp_a_moved_record() {
    let pubkey = "moved-failure-agent";
    let store = Arc::new(Mutex::new(vec![provider_backed_record(
        pubkey, "model-v1", "t1",
    )]));
    let world = World::new("persona-v1");

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();

    let mut effects =
        FakeDeployEffects::new(&store, &world, Err("cluster unreachable".to_string()))
            .gated(started_tx, release_rx);
    let task = tokio::spawn(async move { provider_access::run_deploy(pubkey, &mut effects).await });
    started_rx
        .await
        .expect("failing deploy entered the provider");

    // The agent row is untouched; only the world moves — the same case the
    // success path now catches, applied to the failure path.
    world.set("persona-v2");

    release_tx.send(()).expect("release deploy");
    let error = task
        .await
        .expect("join deploy")
        .expect_err("a failed deploy is not success");
    assert_eq!(
        error, "cluster unreachable",
        "the owner must see the failure"
    );

    let record = store.lock().unwrap()[0].clone();
    assert_eq!(
        record.last_error, None,
        "a failure of the previous configuration was stamped on the current one"
    );
    assert_eq!(record.updated_at, "t1", "an unstamped failure wrote anyway");
}

/// A provider failure whose record switched backends: the failure describes a
/// provider the record no longer names, so nothing is stamped.
#[tokio::test]
async fn provider_failure_does_not_stamp_a_switched_backend() {
    use crate::managed_agents::BackendKind;

    let pubkey = "switched-failure-agent";
    let store = Arc::new(Mutex::new(vec![provider_backed_record(
        pubkey, "model-v1", "t1",
    )]));
    let world = World::new("persona-v1");

    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();

    let mut effects = FakeDeployEffects::new(&store, &world, Err("quota exceeded".to_string()))
        .gated(started_tx, release_rx);
    let task = tokio::spawn(async move { provider_access::run_deploy(pubkey, &mut effects).await });
    started_rx
        .await
        .expect("failing deploy entered the provider");

    {
        let mut records = store.lock().unwrap();
        records[0].backend = BackendKind::Provider {
            id: "other-provider".to_string(),
            config: serde_json::json!({}),
        };
    }

    release_tx.send(()).expect("release deploy");
    let error = task
        .await
        .expect("join deploy")
        .expect_err("a failed deploy is not success");
    assert_eq!(error, "quota exceeded");
    assert_eq!(
        store.lock().unwrap()[0].last_error,
        None,
        "one provider's failure was stamped onto another provider's record"
    );
}

/// Direct unit coverage of the completion-binding branches.
#[test]
fn redeploy_completion_binding_branches() {
    use provider_access::{bind_completion, CompletionOutcome};

    // A digest resolver standing in for the world outside the agent row.
    let resolve = |world: &'static str| {
        move |record: &ManagedAgentRecord| {
            provider_access::payload_digest::deploy_input_digest(
                record,
                &fake_payload(record, world),
            )
        }
    };
    let captured = |record: &ManagedAgentRecord, world: &'static str| {
        let payload = fake_payload(record, world);
        provider_access::redeploy_provider_target(&record.pubkey.clone(), record, &payload)
            .expect("captured")
    };

    // Unchanged: success persisted as current, last_error cleared.
    let mut records = vec![provider_backed_record("agent", "model-v1", "t1")];
    let capture = captured(&records[0], "w1");
    records[0].last_error = Some("old failure".to_string());
    let (save, outcome) = bind_completion(
        &mut records,
        "agent",
        &capture,
        &resolve("w1"),
        &Ok("deploy-1".to_string()),
    );
    assert!(save);
    assert!(matches!(outcome, CompletionOutcome::AppliedCurrent(_)));
    assert_eq!(records[0].backend_agent_id.as_deref(), Some("deploy-1"));
    assert_eq!(records[0].last_error, None);
    assert!(records[0].last_started_at.is_some());

    // Agent row changed mid-flight: remote facts persisted, but nothing implies
    // currency — last_error stays.
    let mut records = vec![provider_backed_record("agent", "model-v2", "t2")];
    let capture = captured(&provider_backed_record("agent", "model-v1", "t1"), "w1");
    records[0].last_error = Some("earlier failure".to_string());
    let (save, outcome) = bind_completion(
        &mut records,
        "agent",
        &capture,
        &resolve("w1"),
        &Ok("deploy-2".to_string()),
    );
    assert!(save);
    assert!(matches!(outcome, CompletionOutcome::AppliedStale));
    assert_eq!(records[0].backend_agent_id.as_deref(), Some("deploy-2"));
    assert_eq!(records[0].last_error.as_deref(), Some("earlier failure"));

    // World changed with the agent row untouched: same `updated_at`, different
    // resolved payload — still not current.
    let mut records = vec![provider_backed_record("agent", "model-v1", "t1")];
    let capture = captured(&records[0], "w1");
    let (save, outcome) = bind_completion(
        &mut records,
        "agent",
        &capture,
        &resolve("w2"),
        &Ok("deploy-3".to_string()),
    );
    assert!(save);
    assert!(
        matches!(outcome, CompletionOutcome::AppliedStale),
        "an input resolved outside the agent row was ignored"
    );

    // Deleted: nothing to write, nothing resurrected.
    let mut records: Vec<ManagedAgentRecord> = vec![];
    let capture = captured(&provider_backed_record("agent", "model-v1", "t1"), "w1");
    let (save, outcome) = bind_completion(
        &mut records,
        "agent",
        &capture,
        &resolve("w1"),
        &Ok("deploy-4".to_string()),
    );
    assert!(!save);
    assert!(matches!(outcome, CompletionOutcome::Deleted));
    assert!(records.is_empty());

    // Moved to a different provider mid-flight: the deploy landed on the old
    // provider, so its id must not be stamped onto the re-homed record.
    let mut records = vec![provider_backed_record("agent", "model-v1", "t2")];
    let capture = captured(&provider_backed_record("agent", "model-v1", "t2"), "w1");
    records[0].backend = crate::managed_agents::BackendKind::Provider {
        id: "other-provider".to_string(),
        config: serde_json::json!({}),
    };
    let (save, outcome) = bind_completion(
        &mut records,
        "agent",
        &capture,
        &resolve("w1"),
        &Ok("deploy-5".to_string()),
    );
    assert!(!save);
    assert!(matches!(outcome, CompletionOutcome::BackendChanged));
    assert_eq!(records[0].backend_agent_id.as_deref(), Some("existing"));
    assert_eq!(records[0].updated_at, "t2");

    // The provider config alone changed: it rides beside the payload on the
    // wire, so it must move the binding too.
    let mut records = vec![provider_backed_record("agent", "model-v1", "t1")];
    let capture = captured(&records[0], "w1");
    records[0].backend = crate::managed_agents::BackendKind::Provider {
        id: "provider".to_string(),
        config: serde_json::json!({"region": "elsewhere"}),
    };
    let (save, outcome) = bind_completion(
        &mut records,
        "agent",
        &capture,
        &resolve("w1"),
        &Ok("deploy-6".to_string()),
    );
    assert!(save);
    assert!(
        matches!(outcome, CompletionOutcome::AppliedStale),
        "a changed provider config was reported as applied"
    );

    // Provider failure with the same record and world: failure stamped, id
    // untouched.
    let mut records = vec![provider_backed_record("agent", "model-v1", "t1")];
    let capture = captured(&records[0], "w1");
    let (save, outcome) = bind_completion(
        &mut records,
        "agent",
        &capture,
        &resolve("w1"),
        &Err("boom".to_string()),
    );
    assert!(save);
    assert!(matches!(outcome, CompletionOutcome::Failed(_)));
    assert_eq!(records[0].last_error.as_deref(), Some("boom"));
    assert_eq!(records[0].backend_agent_id.as_deref(), Some("existing"));

    // Provider failure whose record has moved: nothing stamped at all.
    let mut records = vec![provider_backed_record("agent", "model-v1", "t1")];
    let capture = captured(&records[0], "w1");
    let (save, outcome) = bind_completion(
        &mut records,
        "agent",
        &capture,
        &resolve("w2"),
        &Err("boom".to_string()),
    );
    assert!(!save);
    assert!(matches!(outcome, CompletionOutcome::FailedUnstamped(_)));
    assert_eq!(records[0].last_error, None);
    assert_eq!(records[0].updated_at, "t1");

    // Provider failure that cannot be re-resolved: also unstamped — an
    // unprovable match is not a match.
    let mut records = vec![provider_backed_record("agent", "model-v1", "t1")];
    let capture = captured(&records[0], "w1");
    let (save, outcome) = bind_completion(
        &mut records,
        "agent",
        &capture,
        &|_record| Err("persona is missing".to_string()),
        &Err("boom".to_string()),
    );
    assert!(!save);
    assert!(matches!(outcome, CompletionOutcome::FailedUnstamped(_)));
    assert_eq!(records[0].last_error, None);
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
        error.contains("sent the previous settings"),
        "unexpected error: {error}"
    );

    // Unverified: the deploy happened and currency could not be checked.
    let error = completion_result(
        CompletionOutcome::AppliedUnverified("persona is missing".to_string()),
        Ok("deploy-3".to_string()),
    )
    .expect_err("an unverified completion is not success");
    assert!(
        error.contains("could not re-read") && error.contains("persona is missing"),
        "unexpected error: {error}"
    );

    // Backend switched away mid-flight: honest orphan warning.
    let error = completion_result(
        CompletionOutcome::BackendChanged,
        Ok("deploy-4".to_string()),
    )
    .expect_err("backend change is not success");
    assert!(
        error.contains("backend changed") && error.contains("may still be running"),
        "unexpected error: {error}"
    );

    // Deleted mid-flight: orphan warning on a successful deploy, the provider's
    // own error on a failed one.
    let error = completion_result(CompletionOutcome::Deleted, Ok("deploy-5".to_string()))
        .expect_err("deleted agent is not success");
    assert!(
        error.contains("deleted while the deploy") && error.contains("may still be running"),
        "unexpected error: {error}"
    );
    let error = completion_result(CompletionOutcome::Deleted, Err("boom".to_string()))
        .expect_err("deleted agent is not success");
    assert_eq!(error, "boom");

    // Provider failure: surfaced verbatim, stamped or not.
    let error = completion_result(
        CompletionOutcome::Failed("kaput".to_string()),
        Err("kaput".to_string()),
    )
    .expect_err("failed deploy is not success");
    assert_eq!(error, "kaput");
    let error = completion_result(
        CompletionOutcome::FailedUnstamped("kaput".to_string()),
        Err("kaput".to_string()),
    )
    .expect_err("failed deploy is not success");
    assert_eq!(error, "kaput");
}
