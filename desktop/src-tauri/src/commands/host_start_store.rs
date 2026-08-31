//! Encrypted transport outbox with immutable command and observation payloads.
//! Only a receipt transport timestamp/signature may be renewed. Authority remains in the
//! per-placement journal; transport retry must never create a new operation.
use nostr::Event;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};
use tauri::AppHandle;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Pending {
    pub registration: Event,
    pub command: Event,
    pub receipt: Option<Event>,
    pub published: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub supersedes: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    pub sent: BTreeMap<String, Pending>,
    pub received: BTreeMap<String, Pending>,
    #[serde(default)]
    pub moves: BTreeMap<String, super::host_move::MoveIntent>,
}

pub(super) struct Store {
    _lock: File,
    path: PathBuf,
    pub journal: Journal,
}

impl Store {
    pub fn open(app: &AppHandle, owner: &str, relay: &str) -> Result<Self, String> {
        let dir = crate::managed_agents::managed_agents_base_dir(app)?.join("host-start-outbox");
        Self::open_dir(&dir, owner, relay)
    }

    pub(super) fn open_dir(dir: &Path, owner: &str, relay: &str) -> Result<Self, String> {
        fs::create_dir_all(dir).map_err(|_| "Start outbox unavailable")?;
        let scope = hex::encode(Sha256::digest(format!("{owner}\n{relay}")));
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(dir.join(format!("{scope}.lock")))
            .map_err(|_| "Start outbox lock unavailable")?;
        lock.try_lock()
            .map_err(|_| "Start transport busy; retry shortly")?;
        let path = dir.join(format!("{scope}.json"));
        let journal: Journal = match fs::read(&path) {
            Ok(bytes) if bytes.len() <= 64 * 1024 * 1024 => {
                serde_json::from_slice(&bytes).map_err(|_| "Start outbox corrupt")?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Journal::default(),
            _ => return Err("Start outbox unreadable or full".into()),
        };
        if journal.sent.len() + journal.received.len() + journal.moves.len() > 4096 {
            return Err("Start outbox requires archival".into());
        }
        Ok(Self {
            _lock: lock,
            path,
            journal,
        })
    }

    /// Prepare/authenticate one durable result, fsync its envelope, then publish.
    /// No later entry can age an earlier prepared envelope while doing network
    /// reads. A failed entry remains visible and never starves other placements.
    pub async fn retry_receipts<
        Prepared: std::future::Future<Output = Result<Pending, String>> + Send,
        Published: std::future::Future<Output = Result<(), String>> + Send,
    >(
        &mut self,
        mut prepare: impl FnMut(Pending) -> Prepared,
        mut publish: impl FnMut(Pending) -> Published,
    ) -> Result<(), String> {
        let ids: Vec<_> = self.journal.received.keys().cloned().collect();
        for id in ids {
            let Some(pending) = self.journal.received.get(&id).cloned() else {
                continue;
            };
            if pending.published {
                continue;
            }
            match prepare(pending.clone()).await {
                Ok(mut ready) => {
                    // Even a crash inside publish leaves exactly these signed
                    // bytes durable. Keep the original observation ciphertext.
                    self.journal.received.insert(id.clone(), ready.clone());
                    self.save()?;
                    if !ready.published {
                        match publish(ready.clone()).await {
                            Ok(()) => {
                                ready.published = true;
                                ready.error = None;
                            }
                            Err(error) => ready.error = Some(error),
                        }
                    }
                    self.journal.received.insert(id, ready);
                }
                Err(error) => {
                    let mut failed = pending;
                    failed.error = Some(error);
                    self.journal.received.insert(id, failed);
                }
            }
            self.save()?;
        }
        Ok(())
    }

    pub fn save(&self) -> Result<(), String> {
        if self.journal.sent.len() + self.journal.received.len() + self.journal.moves.len() > 4096 {
            return Err("Start outbox requires archival".into());
        }
        let bytes =
            serde_json::to_vec(&self.journal).map_err(|_| "Start outbox serialization failed")?;
        crate::managed_agents::atomic_write_json_restricted(&self.path, &bytes)
            .map_err(|_| "Start outbox write failed")?;
        #[cfg(unix)]
        File::open(self.path.parent().ok_or("invalid Start outbox path")?)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| "Start outbox sync failed")?;
        Ok(())
    }
}

/// Recover delivery of an already durable observation, never execution. The caller
/// saves the journal BEFORE sending any renewed envelope and still revalidates
/// current registration/owner authority at publication. Keep ciphertext byte-exact:
/// even ambiguous acceptance can only yield the same proof, not a new observation.
pub(super) fn prepare_receipt(
    pending: &mut Pending,
    owner: &nostr::Keys,
    host: &nostr::Keys,
    relay: &str,
    history: &[Event],
    now: u64,
) -> Result<(), String> {
    use buzz_core_pkg::host_execution;
    if pending.published {
        return Ok(());
    }
    // Authenticate the original command at its signed time, including its bounded
    // lifetime. This is read-only evidence recovery, not admission of an expired
    // intent to the executor. Wrong host/community/registration fail closed.
    let request = host_execution::decrypt_command(
        host,
        &pending.registration,
        &pending.command,
        relay,
        pending.command.created_at.as_secs(),
    )?;
    let event = pending
        .receipt
        .as_ref()
        .ok_or("missing durable Start receipt")?;
    host_execution::decrypt_receipt(
        owner,
        &pending.registration,
        event,
        &pending.command,
        &request,
    )?;
    if event.created_at.as_secs() > now.saturating_add(30) {
        return Err("receipt timestamp is in the future".into());
    }
    // ACK loss is not non-acceptance. Match immutable ciphertext/routing, not only
    // event ID: an earlier envelope may have committed before a later retry.
    if history.iter().any(|accepted| {
        accepted.content == event.content
            && accepted.tags == event.tags
            && host_execution::decrypt_receipt(
                owner,
                &pending.registration,
                accepted,
                &pending.command,
                &request,
            )
            .is_ok()
    }) {
        pending.published = true;
        pending.error = None;
        return Ok(());
    }
    // Renew before the relay's +/-900s admission window, leaving time for I/O.
    // Recent retries reuse exact bytes; observation/request/run remain unchanged.
    if now.saturating_sub(event.created_at.as_secs()) >= 600 {
        pending.receipt = Some(
            nostr::EventBuilder::new(event.kind, event.content.clone())
                .allow_self_tagging()
                .tags(event.tags.clone())
                .custom_created_at(nostr::Timestamp::from(now))
                .sign_with_keys(host)
                .map_err(|_| "receipt transport signing failed")?,
        );
    }
    Ok(())
}

/// Attempt every eligible entry independently. Failed publication remains pending
/// and visible; an ACK is recorded only for that exact event. Caller fsyncs before
/// returning. Receipt preparation is persisted separately before this send loop.
pub(super) async fn retry_pending<Fut: std::future::Future<Output = Result<(), String>> + Send>(
    entries: &mut BTreeMap<String, Pending>,
    receipts: bool,
    mut attempt: impl FnMut(Pending) -> Fut,
) {
    for pending in entries.values_mut() {
        if (receipts && pending.published) || (!receipts && pending.receipt.is_some()) {
            continue;
        }
        match attempt(pending.clone()).await {
            Ok(()) => {
                pending.published = true;
                pending.error = None;
            }
            Err(error) => pending.error = Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core_pkg::{
        host,
        host_execution::{self, Action, Command},
    };
    use nostr::{JsonUtil, Keys};

    fn pending() -> Pending {
        let owner = Keys::generate();
        let host = Keys::generate();
        let registration = host::registration(&owner, host.public_key(), 100).unwrap();
        let request = Command {
            v: 1,
            operation: "ab".repeat(16),
            relay: "wss://relay.example".into(),
            agent: Keys::generate().public_key().to_hex(),
            expires_at: 400,
            action: Action::Start {
                runtime: "goose".into(),
                revision: "cd".repeat(32),
            },
        };
        let command = host_execution::command(&owner, &registration, &request, 100).unwrap();
        Pending {
            registration,
            command,
            receipt: None,
            published: false,
            error: None,
            supersedes: None,
        }
    }

    #[tokio::test]
    async fn revoked_entry_and_dropped_ack_do_not_starve_later_intents_and_restart_reuses_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_dir(dir.path(), "owner", "relay").unwrap();
        let old = pending();
        let later = pending();
        let exact = later.command.as_json();
        store.journal.sent.insert("a-revoked".into(), old);
        store.journal.sent.insert("b-current".into(), later);
        store.save().unwrap();
        let attempts = std::sync::atomic::AtomicUsize::new(0);
        retry_pending(&mut store.journal.sent, false, |_| async {
            let count = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(if count == 0 { "revoked" } else { "ACK dropped" }.into())
        })
        .await;
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
        store.save().unwrap();
        assert!(
            Store::open_dir(dir.path(), "owner", "relay").is_err(),
            "cross-process lock"
        );
        drop(store);
        let mut store = Store::open_dir(dir.path(), "owner", "relay").unwrap();
        assert_eq!(
            store.journal.sent["b-current"].error.as_deref(),
            Some("ACK dropped")
        );
        assert_eq!(store.journal.sent["b-current"].command.as_json(), exact);
        retry_pending(&mut store.journal.sent, false, |p| {
            let exact = &exact;
            async move {
                if p.command.as_json() == *exact {
                    Ok(())
                } else {
                    Err("revoked".into())
                }
            }
        })
        .await;
        assert!(store.journal.sent["b-current"].published);
        assert!(store.journal.sent["b-current"].error.is_none());
        assert_eq!(
            store.journal.sent["a-revoked"].error.as_deref(),
            Some("revoked")
        );
        store.save().unwrap();
        drop(store);
        let store = Store::open_dir(dir.path(), "owner", "relay").unwrap();
        assert!(store.journal.sent["b-current"].published);
        assert_eq!(store.journal.sent["b-current"].command.as_json(), exact);
        assert!(Store::open_dir(dir.path(), "owner", "other-relay")
            .unwrap()
            .journal
            .sent
            .is_empty());
    }

    #[test]
    fn malformed_outbox_is_never_replaced_by_an_empty_journal() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_dir(dir.path(), "owner", "relay").unwrap();
        let path = store.path.clone();
        drop(store);
        for corrupt in ["{", "{}", r#"{"sent":{},"received":{},"unexpected":true}"#] {
            fs::write(&path, corrupt).unwrap();
            assert!(Store::open_dir(dir.path(), "owner", "relay").is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), corrupt);
        }
    }
}

#[cfg(test)]
#[path = "host_start_recovery_tests.rs"]
mod recovery_tests;
