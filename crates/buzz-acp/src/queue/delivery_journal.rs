//! Opt-in durability for work the queue has accepted, not a relay history cursor.
use super::*;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub(crate) const ENV: &str = "BUZZ_ACP_DELIVERY_JOURNAL";
const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RECORDS: usize = 10_000;
// The host's controlled restart floor may reach back 15 minutes. Relay REQs
// include another five seconds. Never evict IDs inside that replay overlap.
const COMPLETED_RETENTION_SECS: u64 = crate::REPLAY_FLOOR_MAX_AGE_SECS + 5;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct StoredEvent {
    pub(super) channel_id: Uuid,
    admission_order: u64,
    event: Event,
    // Cache only original-message routing, never admission authority or scope.
    // Absent in older v1 snapshots; those retain the bounded lookup fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    edit: Option<ResolvedEdit>,
}

#[derive(Default, Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    agent_pubkey: String,
    relay_url: String,
    pending: BTreeMap<String, StoredEvent>,
    completed: BTreeMap<String, u64>,
}

pub(super) struct DeliveryJournal {
    path: PathBuf,
    _lock: File,
    snapshot: Snapshot,
    pub(super) restoring: BTreeMap<String, StoredEvent>,
    pub(super) in_flight: HashMap<SessionScope, Vec<StoredEvent>>,
    pub(super) error: Option<String>,
}

fn private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(nix::libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    ensure!(file.metadata()?.is_file(), "journal must be a regular file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            file.metadata()?.permissions().mode() & 0o077 == 0,
            "journal must be private (0600)"
        );
    }
    Ok(file)
}

impl DeliveryJournal {
    fn open(path: &Path, agent_pubkey: &str, relay_url: &str) -> Result<Self> {
        ensure!(path.is_absolute(), "delivery journal path must be absolute");
        let parent = path
            .parent()
            .context("delivery journal needs a parent directory")?;
        let metadata = std::fs::symlink_metadata(parent)?;
        ensure!(
            metadata.is_dir(),
            "delivery journal parent must be a directory"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                metadata.permissions().mode() & 0o077 == 0,
                "delivery journal directory must be private (0700)"
            );
        }
        let lock = private_file(&path.with_extension("lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .context("delivery journal is already owned by another process")?;
        let file = private_file(path)?;
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_BYTES,
            "delivery journal exceeds size limit"
        );
        let snapshot: Snapshot = if bytes.is_empty() {
            Snapshot {
                version: 1,
                agent_pubkey: agent_pubkey.into(),
                relay_url: relay_url.into(),
                ..Default::default()
            }
        } else {
            let snapshot: Snapshot =
                serde_json::from_slice(&bytes).context("invalid delivery journal")?;
            ensure!(
                snapshot.version == 1
                    && snapshot.agent_pubkey == agent_pubkey
                    && snapshot.relay_url == relay_url,
                "delivery journal belongs to another agent or relay, or has an unsupported version"
            );
            ensure!(
                snapshot.pending.len() + snapshot.completed.len() <= MAX_RECORDS,
                "delivery journal exceeds record limit"
            );
            for (id, stored) in &snapshot.pending {
                ensure!(
                    *id == stored.event.id.to_hex(),
                    "delivery journal event ID mismatch"
                );
                stored
                    .event
                    .verify()
                    .context("invalid signed event in delivery journal")?;
                ensure!(
                    stored.event.tags.iter().any(|tag| {
                        let values = tag.as_slice();
                        values.first().is_some_and(|v| v == "h")
                            && values
                                .get(1)
                                .is_some_and(|v| Uuid::parse_str(v).ok() == Some(stored.channel_id))
                    }),
                    "delivery journal channel does not match signed event"
                );
                if let Some(edit) = &stored.edit {
                    ensure!(
                        edit_target_id(&stored.event).as_deref()
                            == Some(edit.target_event_id.as_str()),
                        "delivery journal edit target does not match signed event"
                    );
                    let tags = &edit.target_thread_tags;
                    ensure!(
                        tags.root_event_id.is_some() == tags.parent_event_id.is_some()
                            && tags
                                .root_event_id
                                .iter()
                                .chain(&tags.parent_event_id)
                                .all(|id| nostr::EventId::from_hex(id).is_ok()),
                        "delivery journal edit has invalid thread routing"
                    );
                }
            }
            snapshot
        };
        Ok(Self {
            path: path.into(),
            _lock: lock,
            restoring: snapshot.pending.clone(),
            snapshot,
            in_flight: HashMap::new(),
            error: None,
        })
    }

    fn write(&mut self, pending: BTreeMap<String, StoredEvent>) -> Result<()> {
        let now = nostr::Timestamp::now().as_secs();
        let oldest = now.saturating_sub(COMPLETED_RETENTION_SECS);
        let mut completed = self.snapshot.completed.clone();
        completed.retain(|_, created_at| *created_at >= oldest);
        for (id, stored) in &self.snapshot.pending {
            if !pending.contains_key(id) && stored.event.created_at.as_secs() >= oldest {
                completed.insert(id.clone(), stored.event.created_at.as_secs());
            }
        }
        ensure!(
            pending.len() + completed.len() <= MAX_RECORDS,
            "delivery journal capacity exhausted; refusing to discard replay protection"
        );
        let next = Snapshot {
            version: 1,
            agent_pubkey: self.snapshot.agent_pubkey.clone(),
            relay_url: self.snapshot.relay_url.clone(),
            pending,
            completed,
        };
        let bytes = serde_json::to_vec(&next)?;
        ensure!(
            bytes.len() as u64 <= MAX_BYTES,
            "delivery journal exceeds size limit"
        );
        let parent = self
            .path
            .parent()
            .context("delivery journal needs a parent directory")?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&self.path)?;
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        self.snapshot = next;
        Ok(())
    }
}

impl EventQueue {
    pub(crate) fn with_delivery_journal(
        mut self,
        path: &Path,
        agent: &str,
        relay: &str,
    ) -> Result<Self> {
        self.journal = Some(DeliveryJournal::open(path, agent, relay)?);
        self.checkpoint();
        self.check_persistence()?;
        Ok(self)
    }

    pub(crate) fn check_persistence(&self) -> Result<()> {
        if let Some(error) = self.journal.as_ref().and_then(|j| j.error.as_ref()) {
            anyhow::bail!("delivery journal failed; work dispatch stopped: {error}");
        }
        Ok(())
    }

    pub(crate) fn has_restore_pending(&self) -> bool {
        self.journal
            .as_ref()
            .is_some_and(|j| !j.restoring.is_empty())
    }

    /// Retry only through current channel membership, author and subscription
    /// admission. An unavailable authority lookup must not delete accepted work.
    pub(crate) fn restore_events(
        &self,
        subscribed: &HashSet<Uuid>,
    ) -> VecDeque<crate::relay::BuzzEvent> {
        let mut events: Vec<_> = self
            .journal
            .iter()
            .flat_map(|j| j.restoring.values())
            .filter(|stored| subscribed.contains(&stored.channel_id))
            .map(|stored| {
                (
                    stored.admission_order,
                    crate::relay::BuzzEvent {
                        connection_generation: 0,
                        channel_id: stored.channel_id,
                        event: stored.event.clone(),
                    },
                )
            })
            .collect();
        events.sort_by_key(|(order, event)| (event.event.created_at, *order));
        events.into_iter().map(|(_, event)| event).collect()
    }

    pub(crate) fn delivery_already_accepted(&self, id: &str) -> bool {
        self.journal.as_ref().is_some_and(|j| {
            j.snapshot.completed.contains_key(id)
                || (j.snapshot.pending.contains_key(id) && !j.restoring.contains_key(id))
        })
    }

    /// Preserve admission-time edit routing after current authorization and
    /// subscription checks. Older records without routing still use lookup.
    pub(crate) fn restored_edit(&self, event: &Event) -> Option<ResolvedEdit> {
        self.journal
            .as_ref()?
            .restoring
            .get(&event.id.to_hex())?
            .edit
            .clone()
    }

    pub(super) fn journal_at_capacity(&self, event: &QueuedEvent) -> bool {
        self.journal.as_ref().is_some_and(|journal| {
            !journal.restoring.contains_key(&event.event.id.to_hex())
                && journal
                    .snapshot
                    .pending
                    .values()
                    .filter(|stored| stored.channel_id == event.channel_id)
                    .count()
                    >= MAX_PENDING_PER_CHANNEL
        })
    }

    // A deadline is not evidence that an old worker stopped. Preserve its
    // accepted payload and stop the process before a successor can redeliver.
    pub(super) fn journal_expired(&mut self, scope: &SessionScope) -> bool {
        if let Some(journal) = &mut self.journal {
            if journal.in_flight.contains_key(scope) {
                journal.error = Some(
                    "in-flight delivery expired without terminal settlement; restart required"
                        .into(),
                );
                return true;
            }
        }
        false
    }

    pub(super) fn remember_in_flight<'a>(
        &mut self,
        scope: &SessionScope,
        events: impl Iterator<Item = &'a BatchEvent>,
    ) {
        if let Some(journal) = &mut self.journal {
            journal.in_flight.insert(
                scope.clone(),
                events
                    .filter_map(|event| {
                        journal
                            .snapshot
                            .pending
                            .get(&event.event.id.to_hex())
                            .cloned()
                    })
                    .collect(),
            );
        }
    }

    pub(super) fn checkpoint(&mut self) {
        let Some(journal) = &self.journal else { return };
        if journal.error.is_some() {
            return;
        }
        let mut pending = journal.restoring.clone();
        let mut next_order = journal
            .snapshot
            .pending
            .values()
            .map(|event| event.admission_order)
            .max()
            .unwrap_or(0);
        let mut insert = |channel_id, event: &Event, edit: Option<&ResolvedEdit>| {
            let id = event.id.to_hex();
            let mut stored = journal
                .snapshot
                .pending
                .get(&id)
                .cloned()
                .unwrap_or_else(|| {
                    next_order += 1;
                    StoredEvent {
                        channel_id,
                        admission_order: next_order,
                        event: event.clone(),
                        edit: None,
                    }
                });
            if stored.edit.is_none() {
                stored.edit = edit.cloned();
            }
            pending.insert(id, stored);
        };
        for (scope, events) in &self.queues {
            for event in events {
                insert(scope.channel_id(), &event.event, event.edit.as_ref());
            }
        }
        for (scope, events) in &self.cancelled_batches {
            for event in events {
                insert(scope.channel_id(), &event.event, event.edit.as_ref());
            }
        }
        for (scope, events) in &self.withheld_native_steer {
            for event in events {
                insert(
                    scope.channel_id(),
                    &event.queued.event,
                    event.queued.edit.as_ref(),
                );
            }
        }
        for events in journal.in_flight.values() {
            for event in events {
                insert(event.channel_id, &event.event, event.edit.as_ref());
            }
        }
        if let Some(journal) = &mut self.journal {
            if let Err(error) = journal.write(pending) {
                journal.error = Some(error.to_string());
                tracing::error!("delivery journal persistence failed; stopping dispatch");
            }
        }
    }

    /// Drain completion settles accepted original requests, but a failed or
    /// interrupted task remains on disk for the successor to retry.
    pub(crate) fn settle_shutdown_result(&mut self, result: &crate::pool::PromptResult) {
        use crate::pool::{PromptOutcome, PromptSource};
        let PromptSource::Channel(scope) = &result.source else {
            return;
        };
        let completed = matches!(result.outcome, PromptOutcome::Ok(_))
            || (matches!(
                result.outcome,
                PromptOutcome::Cancelled | PromptOutcome::CancelDrainTimeout(_)
            ) && result.batch.is_none());
        if completed {
            self.finish_native_steers(scope, &result.turn_id, true);
            self.mark_complete(scope);
        }
        tracing::info!(turn_id = %result.turn_id, completed, "delivery journal shutdown settlement");
    }
}

#[cfg(test)]
#[path = "delivery_journal_tests.rs"]
mod tests;
