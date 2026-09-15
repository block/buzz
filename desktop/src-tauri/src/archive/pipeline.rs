//! Archive pipeline — three-phase plan/query/commit split.
//!
//! Separated from `mod.rs` to keep that file under the 1500-line gate.
//!
//! # Send-safety
//!
//! `rusqlite::Connection` is `!Send`. No `&Connection` borrow crosses an
//! `.await` point in any function here. Phase 1 (`plan_archive`) and Phase 3
//! (`prepare::commit_ready`) are sync; query and crypto preparation are async and never
//! holds a `Connection` reference.

use nostr::{Event, JsonUtil};
use rusqlite::Connection;

use crate::active_user_signer::ActiveUserSigner;
use crate::app_state::AppState;
use crate::relay::query_relay_at_with_signer;

use super::{store, validate_ephemeral_public, ArchiveCandidate, MatchedScope};

// ── Private helpers ───────────────────────────────────────────────────────────

/// Extract the raw `kind` integer from an event JSON string and return it as
/// `Some(u64)` only if it is in the valid NIP-01 range `0..=65535`.
///
/// Returns `None` for malformed JSON, a missing `kind` field, a non-integer
/// `kind`, or any value outside `0..=65535`.  Used to detect the `nostr`
/// crate's silent `v as u16` truncation before deserialization.
fn raw_kind_value(raw: &str) -> Option<u64> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let kind = v.get("kind")?.as_u64()?;
    if kind > 65535 {
        return None;
    }
    Some(kind)
}

// ── Private types ────────────────────────────────────────────────────────────

/// A parsed, sig-verified candidate ready for further processing.
pub(crate) struct Parsed {
    pub(crate) event: Event,
    pub(crate) raw_json: String,
    pub(crate) matched_scope: MatchedScope,
}

/// One scope bucket: a set of candidates that share a scope type+value,
/// with the relay filter already built and the subscription kinds loaded.
pub(crate) struct Bucket {
    pub(crate) scope_type_str: String,
    pub(crate) scope_value: String,
    pub(crate) allowed_kinds: Vec<u64>,
    pub(crate) filter: serde_json::Value,
    pub(crate) group: Vec<Parsed>,
}

/// Output of the sync planning phase.
pub(crate) struct ArchivePlan {
    pub(crate) buckets: Vec<Bucket>,
    pub(crate) ephemeral: Vec<Parsed>,
    /// Events already accounted as dropped during planning (no subscription,
    /// unknown scope type, parse failure, bad sig).
    pub(crate) pre_dropped: u32,
}

/// A bucket with the relay's response attached.
pub(crate) struct BucketWithResult {
    pub(crate) scope_type_str: String,
    pub(crate) scope_value: String,
    pub(crate) allowed_kinds: Vec<u64>,
    pub(crate) group: Vec<Parsed>,
    /// Event ids returned by the relay for the scoped filter.
    pub(crate) returned_ids: std::collections::HashSet<String>,
    /// True if the relay query failed; retain the entire group for retry.
    pub(crate) relay_failed: bool,
}

// ── Phase 1 ──────────────────────────────────────────────────────────────────

/// Phase 1 (sync): parse all candidates, group persistent ones into per-scope
/// buckets, and load the subscription kinds for each bucket.
///
/// Returns an [`ArchivePlan`] with no `&Connection` remaining — safe to hold
/// across `.await`.
pub(crate) fn plan_archive(
    candidates: Vec<ArchiveCandidate>,
    identity_pk: &str,
    relay_url: &str,
    conn: &Connection,
) -> Result<ArchivePlan, String> {
    let mut persistent_raw: Vec<Parsed> = Vec::new();
    let mut ephemeral: Vec<Parsed> = Vec::new();
    let mut pre_dropped: u32 = 0;

    for cand in candidates {
        // Range-validate raw kind before nostr::Event normalizes it.
        // `nostr 0.44.3` `Kind::deserialize` does `v as u16`, which silently
        // truncates e.g. `89736` (= 24200 + 65536) to `24200`. We reject any
        // raw `kind` outside 0..=65535 so the validator never reasons about a
        // truncated kind while persisting the original out-of-range value.
        let raw_kind = match raw_kind_value(&cand.raw_event_json) {
            Some(k) => k,
            None => {
                pre_dropped += 1;
                continue;
            }
        };

        let event = match Event::from_json(&cand.raw_event_json) {
            Ok(e) => e,
            Err(_) => {
                pre_dropped += 1;
                continue;
            }
        };

        // Assert the deserialized kind matches the raw value (paranoia check).
        if event.kind.as_u16() as u64 != raw_kind {
            pre_dropped += 1;
            continue;
        }

        if !event.verify_id() || !event.verify_signature() {
            pre_dropped += 1;
            continue;
        }

        // owner_p scope splits by kind:
        //   kind 24200 (observer frames) → ephemeral path (relay never stores them).
        //   kind 44200 (turn metrics)    → persistent path (relay stores, #p-gated).
        //   Any other kind under owner_p follows the same ephemeral path as 24200
        //   (conservative default for unknowns).
        let is_ephemeral = cand.matched_scope.scope_type.is_ephemeral()
            && raw_kind != super::KIND_AGENT_TURN_METRIC as u64;

        if is_ephemeral {
            if validate_ephemeral_public(&event, identity_pk).is_err() {
                pre_dropped += 1;
                continue;
            }
            // DB errors propagate; they are not evidence of invalid input.
            let allowed = store::get_subscription_kinds(
                conn,
                identity_pk,
                relay_url,
                "owner_p",
                &cand.matched_scope.scope_value,
            )?
            .and_then(|json| serde_json::from_str::<Vec<u64>>(&json).ok())
            .is_some_and(|kinds| kinds.contains(&raw_kind));
            if !allowed {
                pre_dropped += 1;
                continue;
            }
            ephemeral.push(Parsed {
                event,
                raw_json: cand.raw_event_json,
                matched_scope: cand.matched_scope,
            });
        } else {
            persistent_raw.push(Parsed {
                event,
                raw_json: cand.raw_event_json,
                matched_scope: cand.matched_scope,
            });
        }
    }

    // Group persistent candidates by (scope_type, scope_value).
    use std::collections::HashMap;
    let mut scope_groups: HashMap<(String, String), Vec<Parsed>> = HashMap::new();
    for p in persistent_raw {
        let key = (
            p.matched_scope.scope_type.as_str().to_string(),
            p.matched_scope.scope_value.clone(),
        );
        scope_groups.entry(key).or_default().push(p);
    }

    let mut buckets: Vec<Bucket> = Vec::with_capacity(scope_groups.len());
    for ((scope_type_str, scope_value), mut group) in scope_groups {
        // No subscription → drop the whole group.
        let kinds_json = match store::get_subscription_kinds(
            conn,
            identity_pk,
            relay_url,
            &scope_type_str,
            &scope_value,
        )? {
            Some(k) => k,
            None => {
                pre_dropped += group.len() as u32;
                continue;
            }
        };

        let allowed_kinds: Vec<u64> =
            serde_json::from_str::<Vec<u64>>(&kinds_json).unwrap_or_default();

        // Deduplicate by event id within the bucket.
        let mut seen = std::collections::HashSet::new();
        group.retain(|p| seen.insert(p.event.id.to_hex()));

        let ids: Vec<String> = group.iter().map(|p| p.event.id.to_hex()).collect();

        // Build a *scoped* relay filter: ids + scope tag + kinds.
        let filter = match scope_type_str.as_str() {
            "channel_h" => serde_json::json!({
                "ids":   ids,
                "#h":    [&scope_value],
                "kinds": allowed_kinds,
            }),
            "referenced_e" => serde_json::json!({
                "ids":   ids,
                "#e":    [&scope_value],
                "kinds": allowed_kinds,
            }),
            "owner_p" => serde_json::json!({
                "ids":   ids,
                "#p":    [&scope_value],
                "kinds": allowed_kinds,
            }),
            _ => {
                pre_dropped += group.len() as u32;
                continue;
            }
        };

        buckets.push(Bucket {
            scope_type_str,
            scope_value,
            allowed_kinds,
            filter,
            group,
        });
    }

    Ok(ArchivePlan {
        buckets,
        ephemeral,
        pre_dropped,
    })
}

// ── Phase 2 ──────────────────────────────────────────────────────────────────

/// Phase 2 (async): fire one relay query per bucket and collect results.
///
/// `state` is `&AppState` — a `Copy` reference — so no `!Send` value is held
/// across `.await`.
pub(crate) async fn query_buckets(
    buckets: Vec<Bucket>,
    state: &AppState,
    relay_api: &str,
    signer: &ActiveUserSigner,
) -> Vec<BucketWithResult> {
    let mut results: Vec<BucketWithResult> = Vec::with_capacity(buckets.len());
    for bucket in buckets {
        let (returned_ids, relay_failed) = match query_relay_at_with_signer(
            state,
            relay_api,
            &[bucket.filter],
            signer,
            None,
        )
        .await
        {
            Ok(evs) => (evs.iter().map(|e| e.id.to_hex()).collect(), false),
            Err(_) => (std::collections::HashSet::new(), true),
        };
        results.push(BucketWithResult {
            scope_type_str: bucket.scope_type_str,
            scope_value: bucket.scope_value,
            allowed_kinds: bucket.allowed_kinds,
            group: bucket.group,
            returned_ids,
            relay_failed,
        });
    }
    results
}
