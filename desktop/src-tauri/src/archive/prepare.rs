//! Crypto preparation and atomic ready-subset persistence.
//!
//! This is not a retry scheduler or a remote activation boundary. The caller
//! owns original ciphertext until commit succeeds. Remote operational errors
//! return that ciphertext, never a terminal observer/metric processed marker.
//! The sync lifecycle still needs its joined, bounded handoff before remote use.

use rusqlite::{Connection, Transaction, TransactionBehavior};
use zeroize::Zeroizing;

use super::{
    pipeline::{BucketWithResult, Parsed},
    store, ArchiveBatchResult, ArchiveCandidate,
};
use crate::active_user_signer::ActiveUserSigner;

pub(crate) struct PreparedBatch {
    ready: Vec<Ready>,
    pub(crate) retry: Vec<ArchiveCandidate>,
    dropped: u32,
}

struct Ready {
    source: Parsed,
    // Only metrics persist plaintext. Drop zeroizes the serialized prepared
    // body on rollback, cancellation, stale admission, and successful commit.
    metric_json: Option<Zeroizing<String>>,
    observer_channel: Option<String>,
}

fn candidate(source: Parsed) -> ArchiveCandidate {
    ArchiveCandidate {
        raw_event_json: source.raw_json,
        matched_scope: source.matched_scope,
    }
}

fn select_candidates(
    buckets: Vec<BucketWithResult>,
    ephemeral: Vec<Parsed>,
    pre_dropped: u32,
) -> (PreparedBatch, Vec<Parsed>) {
    let mut batch = PreparedBatch {
        ready: Vec::new(),
        retry: Vec::new(),
        dropped: pre_dropped,
    };
    let mut selected = ephemeral;
    for bucket in buckets {
        for source in bucket.group {
            if bucket.relay_failed {
                batch.retry.push(candidate(source));
            } else if source.matched_scope.scope_type.as_str() != bucket.scope_type_str
                || source.matched_scope.scope_value != bucket.scope_value
                || !bucket.returned_ids.contains(&source.event.id.to_hex())
                || !bucket
                    .allowed_kinds
                    .contains(&(source.event.kind.as_u16() as u64))
            {
                batch.dropped += 1;
            } else {
                selected.push(source);
            }
        }
    }
    (batch, selected)
}

fn is_observer(source: &Parsed) -> bool {
    source.event.kind.as_u16() == super::KIND_AGENT_OBSERVER_FRAME
        && source.matched_scope.scope_type == super::ScopeType::OwnerP
}

fn requires_crypto(source: &Parsed) -> bool {
    is_observer(source) || source.event.kind.as_u16() == super::KIND_AGENT_TURN_METRIC
}

/// Consume a definitive body verdict. None means local ciphertext/body invalid,
/// NOT remote operational failure. Observer invalidity keeps baseline raw+NULL;
/// metrics with invalid bodies have no canonical or index row.
fn finish_body(batch: &mut PreparedBatch, source: Parsed, plaintext: Option<&str>) {
    let mut ready = Ready {
        source,
        metric_json: None,
        observer_channel: None,
    };
    match ready.source.event.kind.as_u16() {
        super::KIND_AGENT_TURN_METRIC => {
            let payload = plaintext
                .and_then(|text| {
                    buzz_core_pkg::observer::parse_observer_plaintext::<
                        buzz_core_pkg::agent_turn_metric::AgentTurnMetricPayload,
                    >(text)
                    .ok()
                })
                .filter(|payload| payload.validate().is_ok());
            let Some(json) = payload.and_then(|payload| serde_json::to_string(&payload).ok())
            else {
                batch.dropped += 1;
                return;
            };
            ready.metric_json = Some(Zeroizing::new(json));
        }
        super::KIND_AGENT_OBSERVER_FRAME if is_observer(&ready.source) => {
            ready.observer_channel = plaintext
                .and_then(|text| {
                    buzz_core_pkg::observer::parse_observer_plaintext::<serde_json::Value>(text)
                        .ok()
                })
                .and_then(|v| v.get("channelId")?.as_str().map(str::to_owned));
        }
        _ => {}
    }
    batch.ready.push(ready);
}

/// Prepare one independent event at a time, without a connection/store lock.
/// Every signer error after public checks is retryable unless the backend is
/// the local library (`has_local_crypto` must only identify that backend).
/// In particular HTTP 400/401, malformed response and missing
/// capability are not evidence that the signed record is terminal invalid.
pub(crate) async fn prepare_archive(
    buckets: Vec<BucketWithResult>,
    ephemeral: Vec<Parsed>,
    pre_dropped: u32,
    signer: &ActiveUserSigner,
) -> PreparedBatch {
    let (mut batch, selected) = select_candidates(buckets, ephemeral, pre_dropped);
    for source in selected {
        if signer.check_valid().is_err() {
            batch.retry.push(candidate(source));
            continue;
        }
        if !requires_crypto(&source) {
            finish_body(&mut batch, source, None);
            continue;
        }
        if !buzz_core_pkg::observer::content_looks_like_nip44(&source.event.content) {
            finish_body(&mut batch, source, None);
            continue;
        }
        match signer
            .decrypt_record(&source.event.pubkey, &source.event.content)
            .await
        {
            Ok(text) => finish_body(&mut batch, source, text.as_deref().map(String::as_str)),
            Err(_) => batch.retry.push(candidate(source)),
        }
    }
    batch
}

/// Commit only prepared records, atomically with their processed markers.
/// Acquire SQLite write ownership BEFORE admission, including its busy wait.
/// `admit` must check captured identity/generation/relay/lease. This primitive
/// does not serialize logout against commit: remote callers additionally need
/// a lifecycle permit held through COMMIT (remote activation remains closed).
pub(crate) fn commit_ready(
    batch: &PreparedBatch,
    identity_pk: &str,
    relay_url: &str,
    now: i64,
    conn: &Connection,
    admit: impl Fn() -> Result<(), String>,
) -> Result<ArchiveBatchResult, String> {
    commit_ready_guarded(batch, identity_pk, relay_url, now, conn, || Ok(()), admit)
}

/// Acquire a short owner permit only AFTER SQLite write acquisition; keep it
/// through COMMIT. Neither the permit nor a store lock may surround HTTP.
#[allow(clippy::too_many_arguments)]
pub(crate) fn commit_ready_guarded<G>(
    batch: &PreparedBatch,
    identity_pk: &str,
    relay_url: &str,
    now: i64,
    conn: &Connection,
    acquire: impl FnOnce() -> Result<G, String>,
    check: impl Fn() -> Result<(), String>,
) -> Result<ArchiveBatchResult, String> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|e| format!("failed to begin archive transaction: {e}"))?;
    let _permit = acquire()?;
    check()?;
    let mut result = ArchiveBatchResult {
        persisted: 0,
        persisted_agent_metrics: 0,
        dropped: batch.dropped,
    };
    for ready in &batch.ready {
        let p = &ready.source;
        let kind = p.event.kind.as_u16();
        let scope_type = p.matched_scope.scope_type.as_str();
        let scope_value = &p.matched_scope.scope_value;
        // Authorization may have changed during prepare. Re-read after BEGIN
        // IMMEDIATE, never trust the planning snapshot to authorize a write.
        let allowed =
            store::get_subscription_kinds(&tx, identity_pk, relay_url, scope_type, scope_value)?
                .and_then(|json| serde_json::from_str::<Vec<u64>>(&json).ok())
                .is_some_and(|kinds| kinds.contains(&(kind as u64)));
        if !allowed {
            result.dropped += 1;
            continue;
        }
        if is_observer(p) {
            super::validate_ephemeral_frame(
                &p.event,
                identity_pk,
                scope_value,
                &tx,
                identity_pk,
                relay_url,
            )?;
        }
        let eid = p.event.id.to_hex();
        let pubkey = p.event.pubkey.to_hex();
        let created_at = p.event.created_at.as_secs() as i64;
        let raw_json = ready
            .metric_json
            .as_deref()
            .map(String::as_str)
            .unwrap_or(&p.raw_json);
        store::upsert_archived_event(
            &tx,
            identity_pk,
            relay_url,
            &eid,
            kind as i64,
            &pubkey,
            created_at,
            raw_json,
            now,
        )?;
        store::upsert_event_scope(
            &tx,
            identity_pk,
            relay_url,
            &eid,
            scope_type,
            scope_value,
            now,
        )?;
        if kind == super::KIND_AGENT_TURN_METRIC {
            let row = super::metric_store::AgentMetricIndexRow::from_payload(
                raw_json, &eid, &pubkey, created_at, now,
            );
            if super::metric_store::insert_metric_index_row(&tx, identity_pk, relay_url, &row)? {
                result.persisted_agent_metrics += 1;
            }
        }
        if is_observer(p) {
            store::upsert_observer_channel_index(
                &tx,
                identity_pk,
                relay_url,
                &eid,
                ready.observer_channel.as_deref(),
                created_at,
            )?;
        }
        result.persisted += 1;
    }
    check()?;
    tx.commit()
        .map_err(|e| format!("failed to commit archive transaction: {e}"))?;
    Ok(result)
}

#[cfg(test)]
#[path = "prepare_tests.rs"]
mod tests;
