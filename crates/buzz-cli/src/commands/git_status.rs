//! Resolve the currently-effective NIP-34 status (kind:1630-1633) for a root
//! patch / PR / issue event — the read twin of the `status` setter commands
//! added for those kinds.
//!
//! NIP-34's resolution rule is deliberately small: among kind:1630-1633
//! events whose `e`-tag points at the root, only events authored by the **root
//! author** or the **repository owner** count; the latest `created_at` wins,
//! and ties break on the lexicographically-greatest event id.
//!
//! The relay keeps these as ordinary stored events, so resolution is a pure
//! client-side reduce over one bridge query. The fetch is bounded by the same
//! paginated cursor the rest of the CLI uses (`BuzzClient::query_paginated`)
//! because a long-lived root can accumulate an open-ended status history.

use serde_json::Value;

use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::validate_hex64;

/// Kinds that participate in NIP-34 git-point status, as u16 — mirrors
/// `buzz_sdk::GitStatus::kind` without leaking the SDK enum into JSON space.
const GIT_STATUS_KINDS: [u16; 4] = [1630, 1631, 1632, 1633];

/// Hard ceiling on the number of status events we'll fold into one
/// resolution. Chosen well above anything a real root accumulates; the
/// paginated query just stops early when fewer exist.
const STATUS_FETCH_LIMIT: u32 = 1000;

/// The outcome of resolving a root's status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedStatus {
    /// The winning status event, serialized back to the exact JSON the relay
    /// returned (consumers that want every tag — merge commits, `q` refs,
    /// revision links — read them from here).
    pub event: Value,
    /// Convenience mirrors of the winning event's fields, for one-line
    /// rendering and scripting.
    pub kind: u16,
    pub author: String,
    pub created_at: u64,
}

/// Map a status kind u16 to its canonical word — the same vocabulary the
/// setter's `parse_status` accepts (`merged` covers patches AND issues).
pub fn status_word(kind: u16) -> &'static str {
    match kind {
        1630 => "open",
        1631 => "merged",
        1632 => "closed",
        1633 => "draft",
        _ => "unknown",
    }
}

/// Return the winning status event for `root_id` per the NIP-34 rule, or
/// `Ok(None)` when no *authoritative* status exists yet. When `Ok(None)` is
/// returned the implicit state is `open` (NIP-34's default state).
///
/// `root_author` is the pubkey of the root event's author; `repo_owner` is the
/// repo owner pubkey when the caller knows it (patch/issue/PR flows that take
/// `--repo-owner`), otherwise only the root author is eligible.
pub async fn resolve_git_status(
    client: &BuzzClient,
    root_id: &str,
    root_author: &str,
    repo_owner: Option<&str>,
) -> Result<Option<ResolvedStatus>, CliError> {
    pick_winning_status(
        &fetch_status_events(client, root_id).await?,
        root_author,
        repo_owner,
    )
}

/// Fetch every kind:1630-1633 event `e`-tagged to this root, newest-first.
async fn fetch_status_events(
    client: &BuzzClient,
    root_id: &str,
) -> Result<Vec<Value>, CliError> {
    validate_hex64(root_id)?;
    let filter = serde_json::json!({
        "kinds": GIT_STATUS_KINDS,
        "#e": [root_id],
    });
    // query_paginated returns pages in bridge order (newest first per page).
    // We never want the unbounded query_all for user-facing reads.
    client.query_paginated(filter, STATUS_FETCH_LIMIT).await
}

/// Pure reduce implementing the NIP-34 winner rule. Separated from the fetch
/// for unit-testing without a relay: same inputs, same winner.
///
/// - Author must equal `root_author` or `repo_owner` (when provided).
/// - Highest `created_at` wins; ties break on lexicographic id (JS string <
///   on 64-hex is equivalent to byte order, and Rust's Ord on str is
///   byte-wise, so `a.cmp(b)` does the right thing).
/// - Malformed entries (missing/short/implausible fields) are skipped rather
///   than aborting resolution — the relay is authoritative for shape, we
///   treat individual rows as hints.
pub fn pick_winning_status(
    events: &[Value],
    root_author: &str,
    repo_owner: Option<&str>,
) -> Result<Option<ResolvedStatus>, CliError> {
    let mut best: Option<&Value> = None;
    for ev in events {
        let Some(author) = ev.get("pubkey").and_then(Value::as_str) else {
            continue;
        };
        let is_root_author = author == root_author;
        let is_repo_owner = repo_owner.is_some_and(|owner| author == owner);
        if !(is_root_author || is_repo_owner) {
            continue;
        }
        let Some(created_at) = ev.get("created_at").and_then(Value::as_u64) else {
            continue;
        };
        let Some(kind) = ev.get("kind").and_then(Value::as_u64) else {
            continue;
        };
        if !GIT_STATUS_KINDS.contains(&(kind as u16)) {
            continue;
        }
        let Some(id) = ev.get("id").and_then(Value::as_str) else {
            continue;
        };
        let replace = match best {
            None => true,
            Some(cur) => {
                let cur_ts = cur
                    .get("created_at")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let cur_id = cur.get("id").and_then(Value::as_str).unwrap_or("");
                created_at > cur_ts || (created_at == cur_ts && id > cur_id)
            }
        };
        if replace {
            best = Some(ev);
        }
    }
    match best {
        None => Ok(None),
        Some(ev) => {
            let kind = ev
                .get("kind")
                .and_then(Value::as_u64)
                .ok_or_else(|| CliError::Other("status event missing kind".into()))?
                as u16;
            let author = ev
                .get("pubkey")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let created_at = ev
                .get("created_at")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            Ok(Some(ResolvedStatus {
                event: ev.clone(),
                kind,
                author,
                created_at,
            }))
        }
    }
}

/// Resolve and print the current status envelope for a root event. This is
/// the shared body for `patches status-get`, `pr status-get`, and
/// `issues status-get`.
///
/// Output is a single JSON object on stdout:
/// ```json
/// {
///   "root":  "<64-hex>",
///   "state": "open|merged|closed|draft",   // "open" when no status event exists
///   "implicit": true|false,                 // true when state fell back to "open"
///   "kind":  1630|1631|1632|1633|null,
///   "author": "<64-hex>"|null,
///   "created_at": <unix>|null,
///   "event":  <full event JSON>|null
/// }
/// ```
/// `implicit: true` is deliberately loud: script consumers that set status in
/// CI must distinguish "no status has ever been issued" from "a status of
/// open exists". Mirroring the setter, `--repo-owner` is optional; without it
/// only the root author's statuses are eligible (NIP-34's minimal rule).
pub async fn cmd_git_status_get(
    client: &BuzzClient,
    root: &str,
    root_kind: u16,
    repo_owner: Option<&str>,
) -> Result<(), CliError> {
    validate_hex64(root)?;
    if let Some(owner) = repo_owner {
        validate_hex64(owner)?;
    }

    // Fetch the root event itself first — the resolver needs its author, and
    // surfacing "root not found" with a clear error beats silently falling
    // back to `implicit: open` against a typo'd id.
    let root_filter = serde_json::json!({
        "kinds": [root_kind],
        "ids": [root],
    });
    let root_raw = client.query(&root_filter).await?;
    let root_events: Vec<Value> = serde_json::from_str(&root_raw)
        .map_err(|e| CliError::Other(format!("root event parse failed: {e}")))?;
    let root_event = root_events.first().ok_or_else(|| {
        CliError::Other(format!(
            "no kind:{root_kind} root event found with id {root}"
        ))
    })?;
    let root_author = root_event
        .get("pubkey")
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::Other("root event missing pubkey".into()))?;

    match resolve_git_status(client, root, root_author, repo_owner).await? {
        Some(status) => {
            let state = status_word(status.kind);
            let envelope = serde_json::json!({
                "root": root,
                "state": state,
                "implicit": false,
                "kind": status.kind,
                "author": status.author,
                "created_at": status.created_at,
                "event": status.event,
            });
            println!("{}", serde_json::to_string_pretty(&envelope).map_err(|e| {
                CliError::Other(format!("envelope serialize failed: {e}"))
            })?);
        }
        None => {
            // NIP-34 default state: no authoritative status yet ⇒ open,
            // marked implicit so renderers can show it without implying an
            // event exists.
            let envelope = serde_json::json!({
                "root": root,
                "state": "open",
                "implicit": true,
                "kind": null,
                "author": null,
                "created_at": null,
                "event": null,
            });
            println!("{}", serde_json::to_string_pretty(&envelope).map_err(|e| {
                CliError::Other(format!("envelope serialize failed: {e}"))
            })?);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(id: &str, kind: u16, author: &str, created_at: u64) -> Value {
        json!({
            "id": id,
            "kind": kind,
            "pubkey": author,
            "created_at": created_at,
            "content": "",
            "tags": [],
            "sig": "f".repeat(128),
        })
    }

    fn root_author() -> &'static str {
        "a".repeat(64).leak()
    }

    fn repo_owner() -> &'static str {
        "b".repeat(64).leak()
    }

    fn outsider() -> &'static str {
        "c".repeat(64).leak()
    }

    fn id(n: u8) -> String {
        format!("{:064x}", n)
    }

    #[test]
    fn latest_created_at_wins() {
        let events = vec![
            ev(&id(1), 1630, root_author(), 100),
            ev(&id(2), 1631, root_author(), 200), // newest → winner
            ev(&id(3), 1632, root_author(), 150),
        ];
        let got = pick_winning_status(&events, root_author(), Some(repo_owner()))
            .unwrap()
            .expect("a winner exists");
        assert_eq!(got.kind, 1631);
        assert_eq!(got.created_at, 200);
    }

    #[test]
    fn tie_on_created_at_breaks_on_lexicographic_id() {
        let events = vec![
            ev(&id(0x10), 1631, root_author(), 200),
            ev(&id(0x2f), 1632, root_author(), 200), // same ts, larger id → winner
        ];
        let got = pick_winning_status(&events, root_author(), Some(repo_owner()))
            .unwrap()
            .expect("a winner exists");
        assert_eq!(got.kind, 1632);
    }

    #[test]
    fn root_author_and_repo_owner_both_eligible() {
        let events = vec![
            ev(&id(1), 1630, root_author(), 100),
            ev(&id(2), 1632, repo_owner(), 200), // repo owner's newer status wins
        ];
        let got = pick_winning_status(&events, root_author(), Some(repo_owner()))
            .unwrap()
            .expect("a winner exists");
        assert_eq!(got.kind, 1632);
        assert_eq!(got.author, repo_owner());
    }

    #[test]
    fn outsider_events_are_ignored() {
        let events = vec![
            ev(&id(1), 1630, root_author(), 100),
            ev(&id(2), 1632, outsider(), 999), // later, but not authoritative
        ];
        let got = pick_winning_status(&events, root_author(), Some(repo_owner()))
            .unwrap()
            .expect("a winner exists");
        assert_eq!(got.kind, 1630);
    }

    #[test]
    fn no_repo_owner_means_only_root_author_counts() {
        // NIP-34 minimal rule: without the repo-owner hint the reduce can't
        // award status to anyone but the root's own author — mirroring the
        // setter, which tags the owner only when told who it is.
        let events = vec![
            ev(&id(1), 1630, root_author(), 100),
            ev(&id(2), 1632, repo_owner(), 200),
        ];
        let got = pick_winning_status(&events, root_author(), None)
            .unwrap()
            .expect("a winner exists");
        assert_eq!(got.kind, 1630, "owner status must be ineligible without the hint");
    }

    #[test]
    fn empty_history_yields_none() {
        let got = pick_winning_status(&[], root_author(), Some(repo_owner())).unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn events_with_missing_fields_are_skipped() {
        let events = vec![
            json!({"id": id(9), "kind": 1632}),                     // no pubkey/created_at
            json!({"id": id(8), "pubkey": root_author()}),          // no kind/created_at
            json!({"pubkey": root_author(), "kind": 1631, "created_at": 50}), // no id
            ev(&id(3), 1631, root_author(), 50),                    // valid winner
        ];
        let got = pick_winning_status(&events, root_author(), Some(repo_owner()))
            .unwrap()
            .expect("a winner exists");
        assert_eq!(got.kind, 1631);
        assert_eq!(got.event.get("id").unwrap().as_str().unwrap(), id(3));
    }

    #[test]
    fn non_status_kinds_are_skipped() {
        let events = vec![
            ev(&id(1), 1630, root_author(), 100),
            ev(&id(2), 1617, root_author(), 999), // patch, not status
        ];
        let got = pick_winning_status(&events, root_author(), Some(repo_owner()))
            .unwrap()
            .expect("a winner exists");
        assert_eq!(got.kind, 1630);
    }

    #[test]
    fn status_word_maps_all_kinds() {
        assert_eq!(status_word(1630), "open");
        assert_eq!(status_word(1631), "merged");
        assert_eq!(status_word(1632), "closed");
        assert_eq!(status_word(1633), "draft");
    }
}
