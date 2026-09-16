use std::sync::Arc;

use nostr::{
    signer::SignerBackend, util::BoxedFuture, Event, EventBuilder, JsonUtil, Keys, Kind,
    NostrSigner, PublicKey, SignerError, Tag, UnsignedEvent,
};
use tokio::sync::Notify;

use super::*;
use crate::archive::{pipeline::plan_archive, ArchiveCandidate, MatchedScope, ScopeType};

#[derive(Debug)]
struct PausedDecrypt {
    keys: Keys,
    entered: Notify,
    release: Notify,
    fail: bool,
}

macro_rules! delegate {
    ($method:ident) => {
        fn $method<'a>(
            &'a self,
            peer: &'a PublicKey,
            content: &'a str,
        ) -> BoxedFuture<'a, Result<String, SignerError>> {
            self.keys.$method(peer, content)
        }
    };
}

impl NostrSigner for PausedDecrypt {
    fn backend(&self) -> SignerBackend<'_> {
        SignerBackend::Custom("paused-decrypt".into())
    }
    fn get_public_key(&self) -> BoxedFuture<'_, Result<PublicKey, SignerError>> {
        Box::pin(async { Ok(self.keys.public_key()) })
    }
    fn sign_event(&self, event: UnsignedEvent) -> BoxedFuture<'_, Result<Event, SignerError>> {
        NostrSigner::sign_event(&self.keys, event)
    }
    delegate!(nip04_encrypt);
    delegate!(nip04_decrypt);
    delegate!(nip44_encrypt);
    fn nip44_decrypt<'a>(
        &'a self,
        peer: &'a PublicKey,
        content: &'a str,
    ) -> BoxedFuture<'a, Result<String, SignerError>> {
        Box::pin(async move {
            self.entered.notify_one();
            self.release.notified().await;
            if self.fail {
                Err(SignerError::from("backend disconnected"))
            } else {
                self.keys.nip44_decrypt(peer, content).await
            }
        })
    }
}

// Exercises the production preparation/commit seam with a genuinely suspended
// NostrSigner: SQLite remains writable during crypto and a removed subscription
// cannot be resurrected by the delayed result.
#[tokio::test]
async fn delayed_decrypt_rechecks_subscription_before_commit() {
    let controlled = Arc::new(PausedDecrypt {
        keys: Keys::generate(),
        entered: Notify::new(),
        release: Notify::new(),
        fail: false,
    });
    let signer = ActiveUserSigner::new(controlled.clone()).await.unwrap();
    let owner = signer.public_key().to_hex();
    let agent = Keys::generate();
    let content = buzz_core_pkg::observer::encrypt_observer_payload(
        &agent,
        &signer.public_key(),
        &serde_json::json!({"channelId": "channel-a"}),
    )
    .unwrap();
    let event = EventBuilder::new(Kind::Custom(24200), content)
        .tags([
            Tag::public_key(signer.public_key()),
            Tag::parse(["agent", &agent.public_key().to_hex()]).unwrap(),
            Tag::parse(["frame", "telemetry"]).unwrap(),
        ])
        .sign_with_keys(&agent)
        .unwrap();
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(store::SCHEMA).unwrap();
    let relay = "wss://relay.example";
    store::upsert_save_subscription(&conn, &owner, relay, "owner_p", &owner, "[24200]", 0).unwrap();
    let plan = plan_archive(
        vec![ArchiveCandidate {
            raw_event_json: event.as_json(),
            matched_scope: MatchedScope {
                scope_type: ScopeType::OwnerP,
                scope_value: owner.clone(),
            },
        }],
        &owner,
        relay,
        &conn,
    )
    .unwrap();
    let task = tokio::spawn(async move {
        prepare_archive(Vec::new(), plan.ephemeral, plan.pre_dropped, &signer).await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        controlled.entered.notified(),
    )
    .await
    .unwrap();
    assert!(!task.is_finished());
    conn.execute("DELETE FROM save_subscriptions", []).unwrap();
    controlled.release.notify_one();
    let prepared = task.await.unwrap();
    assert_eq!(prepared.ready.len(), 1);
    assert_eq!(
        prepared.ready[0].observer_channel.as_deref(),
        Some("channel-a")
    );
    let result = commit_ready(&prepared, &owner, relay, 0, &conn, || Ok(())).unwrap();
    assert_eq!(result.persisted, 0);
    assert_eq!(result.dropped, 1);
    let count: i64 = conn
        .query_row("SELECT count(*) FROM archived_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn delayed_backend_failure_retains_observer_and_metric_ciphertext() {
    for kind in [24200, 44200] {
        let controlled = Arc::new(PausedDecrypt {
            keys: Keys::generate(),
            entered: Notify::new(),
            release: Notify::new(),
            fail: true,
        });
        let signer = ActiveUserSigner::new(controlled.clone()).await.unwrap();
        let agent = Keys::generate();
        let content = buzz_core_pkg::observer::encrypt_observer_payload(
            &agent,
            &signer.public_key(),
            &serde_json::json!({"channelId": "channel-a"}),
        )
        .unwrap();
        let event = EventBuilder::new(Kind::Custom(kind), content)
            .sign_with_keys(&agent)
            .unwrap();
        let owner = signer.public_key().to_hex();
        let source = Parsed {
            raw_json: event.as_json(),
            matched_scope: MatchedScope {
                scope_type: ScopeType::OwnerP,
                scope_value: owner.clone(),
            },
            event,
        };
        // Both kinds enter via an accepted bucket so the test exercises the
        // same selection/decrypt/finish boundary, not a test-only classifier.
        let bucket = BucketWithResult {
            scope_type_str: "owner_p".into(),
            scope_value: owner,
            allowed_kinds: vec![u64::from(kind)],
            returned_ids: [source.event.id.to_hex()].into_iter().collect(),
            group: vec![source],
            relay_failed: false,
        };
        let original = bucket.group[0].raw_json.clone();
        let plain = EventBuilder::new(Kind::Custom(9), "ready message")
            .sign_with_keys(&agent)
            .unwrap();
        let ready_bucket = BucketWithResult {
            scope_type_str: "channel_h".into(),
            scope_value: "channel-a".into(),
            allowed_kinds: vec![9],
            returned_ids: [plain.id.to_hex()].into_iter().collect(),
            group: vec![Parsed {
                raw_json: plain.as_json(),
                event: plain,
                matched_scope: MatchedScope {
                    scope_type: ScopeType::ChannelH,
                    scope_value: "channel-a".into(),
                },
            }],
            relay_failed: false,
        };
        let task = tokio::spawn(async move {
            prepare_archive(vec![bucket, ready_bucket], Vec::new(), 0, &signer).await
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            controlled.entered.notified(),
        )
        .await
        .unwrap();
        assert!(!task.is_finished());
        controlled.release.notify_one();
        let result = task.await.unwrap();
        assert_eq!(result.ready.len(), 1);
        assert_eq!(result.ready[0].source.event.kind, Kind::Custom(9));
        assert_eq!(result.dropped, 0);
        assert_eq!(result.retry.len(), 1);
        assert_eq!(result.retry[0].raw_event_json, original);
        // Operational failure cannot create a processed observer NULL row or
        // a dropped-metric verdict; the retry owner retains exact ciphertext.
    }
}

fn fixture() -> (Connection, String, PreparedBatch) {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let owner_pk = owner.public_key().to_hex();
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(store::SCHEMA).unwrap();
    store::upsert_save_subscription(
        &conn, &owner_pk, "relay", "owner_p", &owner_pk, "[24200]", 0,
    )
    .unwrap();
    let ciphertext = buzz_core_pkg::observer::encrypt_observer_payload(
        &agent,
        &owner.public_key(),
        &serde_json::json!({"channelId":"channel"}),
    )
    .unwrap();
    let event = EventBuilder::new(Kind::Custom(24200), ciphertext)
        .tags([
            Tag::parse(["p", &owner_pk]).unwrap(),
            Tag::parse(["agent", &agent.public_key().to_hex()]).unwrap(),
            Tag::parse(["frame", "telemetry"]).unwrap(),
        ])
        .sign_with_keys(&agent)
        .unwrap();
    let plan = plan_archive(
        vec![ArchiveCandidate {
            raw_event_json: event.as_json(),
            matched_scope: MatchedScope {
                scope_type: ScopeType::OwnerP,
                scope_value: owner_pk.clone(),
            },
        }],
        &owner_pk,
        "relay",
        &conn,
    )
    .unwrap();
    let signer = ActiveUserSigner::local(owner.clone());
    let batch = tauri::async_runtime::block_on(prepare_archive(vec![], plan.ephemeral, 0, &signer));
    (conn, owner_pk, batch)
}

#[test]
fn ready_subset_rolls_back_all_rows_on_marker_failure_and_can_retry() {
    let (conn, owner, batch) = fixture();
    conn.execute_batch("CREATE TRIGGER fail_marker BEFORE INSERT ON observer_channel_index BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap();
    assert!(commit_ready(&batch, &owner, "relay", 1, &conn, || Ok(())).is_err());
    for table in [
        "archived_events",
        "archived_event_scopes",
        "observer_channel_index",
    ] {
        assert_eq!(
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    conn.execute_batch("DROP TRIGGER fail_marker").unwrap();
    assert_eq!(
        commit_ready(&batch, &owner, "relay", 2, &conn, || Ok(()))
            .unwrap()
            .persisted,
        1
    );
    assert_eq!(
        commit_ready(&batch, &owner, "relay", 3, &conn, || Ok(()))
            .unwrap()
            .persisted,
        1
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM observer_channel_index", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row("SELECT archived_at FROM archived_events", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn final_admission_failure_rolls_back_and_planning_db_error_is_not_terminal_input() {
    let (conn, owner, batch) = fixture();
    let checks = std::cell::Cell::new(0);
    let result = commit_ready(&batch, &owner, "relay", 1, &conn, || {
        checks.set(checks.get() + 1);
        if checks.get() == 2 {
            Err("expired at final boundary".into())
        } else {
            Ok(())
        }
    });
    assert!(result.is_err());
    assert_eq!(checks.get(), 2);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM archived_events", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let source = &batch.ready[0].source;
    let candidates = vec![ArchiveCandidate {
        raw_event_json: source.raw_json.clone(),
        matched_scope: source.matched_scope.clone(),
    }];
    conn.execute_batch("DROP TABLE save_subscriptions").unwrap();
    assert!(plan_archive(candidates, &owner, "relay", &conn).is_err());
}

#[tokio::test]
async fn admission_is_after_sqlite_busy_acquisition_not_before_it() {
    use std::cell::RefCell;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    type BusySignal = (
        tokio::sync::oneshot::Sender<()>,
        std::sync::mpsc::Receiver<()>,
    );
    thread_local! { static BUSY: RefCell<Option<BusySignal>> = const { RefCell::new(None) }; }
    fn busy(_: i32) -> bool {
        BUSY.with(|slot| {
            if let Some((arrived, release)) = slot.borrow_mut().take() {
                let _ = arrived.send(());
                release.recv().unwrap();
            }
        });
        true
    }
    let (_memory, owner, batch) = tokio::task::spawn_blocking(fixture).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("archive.db");
    let writer = store::open_archive_db(&path).unwrap();
    store::upsert_save_subscription(&writer, &owner, "relay", "owner_p", &owner, "[24200]", 0)
        .unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (arrived_tx, arrived) = tokio::sync::oneshot::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let valid = Arc::new(AtomicBool::new(true));
    let check = valid.clone();
    let task = tokio::task::spawn_blocking(move || {
        let conn = Connection::open(path).unwrap();
        BUSY.with(|slot| *slot.borrow_mut() = Some((arrived_tx, release_rx)));
        conn.busy_handler(Some(busy)).unwrap();
        commit_ready(&batch, &owner, "relay", 1, &conn, || {
            if check.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err("revoked during SQLite busy wait".into())
            }
        })
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), arrived)
        .await
        .unwrap()
        .unwrap();
    valid.store(false, Ordering::SeqCst);
    writer.execute_batch("COMMIT").unwrap();
    release.send(()).unwrap();
    assert!(task.await.unwrap().is_err());
    assert_eq!(
        writer
            .query_row("SELECT COUNT(*) FROM archived_events", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
