//! F6 production lifecycle boundaries, not standalone cache invalidation tests.
//! Parent fixture owns only synthetic identities, temporary files and fake children.
use super::*;
use std::sync::{Arc, Mutex};

fn write_raw(fixture: &Fixture, records: &[ManagedAgentRecord]) {
    let path = agents::storage::managed_agents_store_path(fixture.app.handle()).unwrap();
    agents::storage::atomic_write_json_restricted(&path, &serde_json::to_vec(records).unwrap())
        .unwrap();
}

fn raw(fixture: &Fixture) -> ManagedAgentRecord {
    agents::storage::load_agent_store(fixture.app.handle())
        .unwrap()
        .into_iter()
        .find(|r| r.pubkey == fixture.record.pubkey)
        .unwrap()
}

struct Synthetic {
    backend: Arc<Mutex<std::collections::HashMap<String, String>>>,
    store: &'static crate::secret_store::SecretStore,
    name: String,
    _guard: agents::storage::TestAgentSecretStore,
}
impl Synthetic {
    fn warm(fixture: &Fixture) -> Self {
        let name = format!("agent:{}", fixture.record.pubkey);
        let backend = Arc::new(Mutex::new(std::collections::HashMap::from([
            (name.clone(), fixture.record.private_key_nsec.clone()),
            ("identity".into(), "unrelated-synthetic-identity".into()),
            ("agent:other".into(), "rotated-synthetic-key".into()),
        ])));
        // Bounded to this fixture suite; the override requires a static store,
        // and is always removed by its guard before another fixture runs.
        let store = Box::leak(Box::new(crate::secret_store::SecretStore::synthetic(
            backend.clone(),
        )));
        assert_eq!(
            store.load(&name).unwrap(),
            Some(fixture.record.private_key_nsec.clone())
        );
        let mut record = raw(fixture);
        record.private_key_nsec.clear();
        write_raw(fixture, &[record]);
        Self {
            backend,
            store,
            name,
            _guard: agents::storage::TestAgentSecretStore::install(store),
        }
    }
    fn revoke(&self) {
        self.backend.lock().unwrap().remove(&self.name);
        // A fresh read MUST NOT be mistaken for cache invalidation as the fix.
        assert!(self
            .store
            .load_fresh_readonly(&self.name)
            .unwrap()
            .is_none());
        assert!(self.store.load(&self.name).unwrap().is_some());
    }
    fn assert_absent(&self, fixture: &Fixture) {
        let map = self.backend.lock().unwrap();
        assert!(!map.contains_key(&self.name));
        assert_eq!(map.get("identity").unwrap(), "unrelated-synthetic-identity");
        assert_eq!(map.get("agent:other").unwrap(), "rotated-synthetic-key");
        assert!(raw(fixture).private_key_nsec.is_empty());
    }
}

// Acquire the process-environment lock outside the executor: no synchronous
// mutex acquisition can block a competing async fixture's executor thread.
fn with_path_lock(test: impl std::future::Future<Output = ()>) {
    let _guard = agents::lock_path_mutex();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(test);
}

async fn assert_ineligible(fixture: &Fixture) {
    let catalog = fixture.action(Action::Catalog, None).await;
    assert!(
        !catalog
            .observation
            .unwrap()
            .catalog
            .unwrap()
            .entry
            .unwrap()
            .eligible
    );
    assert_eq!(
        fixture.action(Action::Start, None).await.outcome,
        Outcome::Ineligible
    );
    assert!(fixture.running().is_none());
}

#[test]
fn warm_external_revocation_survives_ordinary_signed_stop() {
    with_path_lock(async {
        let fixture = Fixture::new();
        let backend = Synthetic::warm(&fixture);
        assert_eq!(
            fixture.action(Action::Start, None).await.outcome,
            Outcome::Running
        );
        fixture.launched("fixture-model|fixture-model");
        let pid = fixture.running().unwrap().0;
        backend.revoke();
        let catalog = fixture.action(Action::Catalog, None).await;
        assert!(
            !catalog
                .observation
                .unwrap()
                .catalog
                .unwrap()
                .entry
                .unwrap()
                .eligible
        );
        assert_eq!(fixture.running().unwrap().0, pid);
        assert_eq!(fixture.stop().await, StopOutcome::Stopped);
        assert!(!agents::process_is_running(pid));
        backend.assert_absent(&fixture);
        assert_ineligible(&fixture).await;
        backend.assert_absent(&fixture);
    });
}

#[test]
fn inline_revocation_during_ordinary_stop_survives_same_host_start() {
    with_path_lock(async {
        for launch in ["signed-stop", "local-start", "restore"] {
            let mut fixture = Fixture::new();
            fixture.record.start_on_app_launch = true;
            fixture.persist();
            let path = agents::storage::managed_agents_store_path(fixture.app.handle()).unwrap();
            let revoked_path = fixture.temp.path().join("ordinary-stop-revoked.json");
            let mut revoked = raw(&fixture);
            revoked.private_key_nsec.clear();
            std::fs::write(&revoked_path, serde_json::to_vec(&[revoked]).unwrap()).unwrap();
            on_stop(
                &fixture,
                &format!(
                    "/bin/cp \"{}\" \"{}\"; exit 0",
                    revoked_path.display(),
                    path.display()
                ),
            );
            assert_eq!(
                fixture.action(Action::Start, None).await.outcome,
                Outcome::Running
            );
            fixture.launched("fixture-model|fixture-model");
            let pid = fixture.running().unwrap().0;
            if launch == "signed-stop" {
                assert_eq!(fixture.stop().await, StopOutcome::Stopped);
            } else {
                // Simulate a previous Desktop session: keep only the durable receipt.
                // Teardown now happens INSIDE ordinary local Start / restore, after
                // preflight has captured the still-valid inline credential.
                let state = fixture.app.state::<crate::app_state::AppState>();
                let key =
                    agents::ManagedAgentRuntimeKey::new(&fixture.record.pubkey, COMMUNITY).unwrap();
                let mut old_runtime = state
                    .managed_agent_processes
                    .lock()
                    .unwrap()
                    .remove(&key)
                    .unwrap();
                if launch == "local-start" {
                    assert!(crate::commands::start_local_agent_with_preflight_using(
                        fixture.app.handle(),
                        &state,
                        &fixture.record.pubkey,
                        crate::commands::LocalStartIntent::Explicit,
                        Some(COMMUNITY),
                        Some(&fixture.owner.public_key().to_hex()),
                        None,
                        Some(Some(&fixture.named.reference())),
                        |_, _| async { Ok(()) },
                    )
                    .await
                    .is_err());
                } else {
                    agents::restore_with(
                        fixture.app.handle(),
                        &std::sync::atomic::AtomicBool::new(false),
                        |_, _| async { Ok(()) },
                        |_| {},
                        |_, _| panic!("post-teardown revoked restore must not publish"),
                    )
                    .await
                    .unwrap();
                }
                // Reap the synthetic child even though Desktop no longer tracks it.
                let _ = agents::terminate_process(old_runtime.child.id());
                let _ = old_runtime.child.wait();
            }
            assert!(!agents::process_is_running(pid));
            assert!(raw(&fixture).private_key_nsec.is_empty());
            assert_ineligible(&fixture).await;
            assert!(raw(&fixture).private_key_nsec.is_empty());
            assert_eq!(
                std::fs::read_to_string(fixture.temp.path().join("launches"))
                    .unwrap()
                    .lines()
                    .count(),
                1
            );
        }
    });
}

#[test]
fn ordinary_local_named_start_cannot_provision_from_warm_cache() {
    with_path_lock(async {
        for before_preparation in [true, false] {
            for preflight_succeeds in [true, false] {
                let fixture = Fixture::new();
                let backend = Synthetic::warm(&fixture);
                if before_preparation {
                    backend.revoke();
                }
                let state = fixture.app.state::<crate::app_state::AppState>();
                let result = crate::commands::start_local_agent_with_preflight_using(
                    fixture.app.handle(),
                    &state,
                    &fixture.record.pubkey,
                    crate::commands::LocalStartIntent::Explicit,
                    Some(COMMUNITY),
                    Some(&fixture.owner.public_key().to_hex()),
                    None,
                    Some(Some(&fixture.named.reference())),
                    |_, _| {
                        if !before_preparation {
                            backend.revoke();
                        }
                        backend.assert_absent(&fixture);
                        async move {
                            if preflight_succeeds {
                                Ok(())
                            } else {
                                Err("synthetic provider refusal".into())
                            }
                        }
                    },
                )
                .await;
                assert!(result.is_err());
                backend.assert_absent(&fixture);
                assert_ineligible(&fixture).await;
                assert!(!fixture.temp.path().join("launches").exists());
            }
        }
    });
}

#[test]
fn bulk_and_restore_preparation_never_restore_cached_credentials() {
    with_path_lock(async {
        for restore in [false, true] {
            let mut fixture = Fixture::new();
            fixture.record.start_on_app_launch = true;
            fixture.persist();
            let backend = Synthetic::warm(&fixture);
            if restore {
                let shutdown = std::sync::atomic::AtomicBool::new(false);
                agents::restore_with(
                    fixture.app.handle(),
                    &shutdown,
                    |_, _| {
                        backend.revoke();
                        async { Ok(()) }
                    },
                    |_| {},
                    |_, _| panic!("revoked restore must not publish"),
                )
                .await
                .unwrap();
            } else {
                let state = fixture.app.state::<crate::app_state::AppState>();
                let result = crate::commands::start_local_agent_pairs_with_preflight_using(
                    fixture.app.handle(),
                    &state,
                    &fixture.record.pubkey,
                    &[COMMUNITY.into()],
                    |_, _| {
                        backend.revoke();
                        async { Ok(()) }
                    },
                )
                .await;
                assert!(result.is_err());
            }
            backend.assert_absent(&fixture);
            assert_ineligible(&fixture).await;
            assert!(!fixture.temp.path().join("launches").exists());
        }
    });
}

#[test]
fn metadata_merge_preserves_rotations_scopes_and_removed_records() {
    with_path_lock(async {
        let fixture = Fixture::new();
        let stale = raw(&fixture);
        let mut current = stale.clone();
        current.private_key_nsec = "deliberately-rotated-inline-fixture".into();
        current.name = "newer definition".into();
        save(
            &mut current,
            &fixture.owner.public_key().to_hex(),
            "wss://other-scope.example",
            config("other-host"),
        );
        let mut other = current.clone();
        other.pubkey = nostr::Keys::generate().public_key().to_hex();
        other.private_key_nsec = "other-inline-fixture".into();
        write_raw(&fixture, &[current.clone(), other.clone()]);
        agents::storage::save_runtime_metadata_batch(
            fixture.app.handle(),
            std::slice::from_ref(&stale),
        )
        .unwrap();
        let saved = raw(&fixture);
        assert_eq!(saved.private_key_nsec, current.private_key_nsec);
        assert_eq!(saved.name, current.name);
        assert_eq!(
            serde_json::to_value(saved.runtime_configurations).unwrap(),
            serde_json::to_value(current.runtime_configurations).unwrap()
        );
        assert_eq!(
            agents::storage::load_agent_store(fixture.app.handle()).unwrap()[1].private_key_nsec,
            other.private_key_nsec
        );
        // Ordinary edits also have no key-write authority; a stale key cannot undo
        // a legitimate rotation, and no missing identity is implicitly provisioned.
        agents::save_managed_agents(fixture.app.handle(), &[stale.clone(), other.clone()]).unwrap();
        assert_eq!(
            raw(&fixture).private_key_nsec,
            "deliberately-rotated-inline-fixture"
        );
        write_raw(&fixture, &[other]);
        agents::storage::save_runtime_metadata_batch(
            fixture.app.handle(),
            std::slice::from_ref(&stale),
        )
        .unwrap();
        assert_eq!(
            agents::storage::load_agent_store(fixture.app.handle())
                .unwrap()
                .len(),
            1
        );
        assert!(agents::save_managed_agents(fixture.app.handle(), &[stale]).is_err());
    });
}

#[test]
fn deliberate_new_identity_provisioning_does_not_rewrite_existing_keys() {
    with_path_lock(async {
        let fixture = Fixture::new();
        let backend = Synthetic::warm(&fixture);
        backend.revoke();
        let new_record = record();
        let new_name = format!("agent:{}", new_record.pubkey);
        assert_ne!(new_record.pubkey, fixture.record.pubkey);
        let records = [fixture.record.clone(), new_record.clone()];
        assert!(agents::save_managed_agents(fixture.app.handle(), &records).is_err());
        backend.assert_absent(&fixture);
        agents::storage::save_managed_agents_with_new_keys(fixture.app.handle(), &records).unwrap();
        backend.assert_absent(&fixture);
        assert_eq!(
            backend.store.load_fresh_readonly(&new_name).unwrap(),
            Some(new_record.private_key_nsec)
        );
        assert!(agents::storage::load_agent_store(fixture.app.handle())
            .unwrap()
            .iter()
            .all(|r| r.private_key_nsec.is_empty()));
    });
}
