//! Relay plumbing shared by `buzz personas` and `buzz teams`.
//!
//! Persona (kind:30175) and team (kind:30176) events are NIP-33
//! parameterized-replaceable and authored by the OWNER. The CLI identity is
//! therefore the owner itself — unlike `buzz agents draft-*`, which is an agent
//! asking an owner to act, no NIP-OA auth tag is involved. Run these commands
//! with the same key as the Buzz Desktop you expect the definitions to appear
//! in; a different key publishes to a different coordinate space.

use std::time::{SystemTime, UNIX_EPOCH};

use buzz_sdk::agent_definitions::event_d_tag;
use nostr::{EventBuilder, JsonUtil};

use crate::client::BuzzClient;
use crate::error::CliError;

/// Maximum definitions fetched per listing query.
pub const LIST_LIMIT: usize = 500;

/// Relay ceiling on event content, mirroring `MAX_EVENT_CONTENT_BYTES` in
/// `buzz-relay`'s ingest path.
pub const MAX_EVENT_CONTENT_LEN: usize = 256 * 1024;

/// Seconds since the Unix epoch, saturating at 0 on a pre-epoch clock.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Parse a relay-response JSON array of events, discarding entries that fail to
/// deserialize so one corrupt record can't deny-of-service a whole listing.
///
/// Returns the parsed events and how many the relay actually sent. Callers that
/// test against a query limit must use the returned count, not the vector
/// length, or a dropped record hides a real truncation.
fn parse_events(json: &str) -> Result<(Vec<nostr::Event>, usize), CliError> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| CliError::Other(format!("relay returned invalid JSON: {e}")))?;
    let arr = value
        .as_array()
        .ok_or_else(|| CliError::Other("relay response is not an array".into()))?;
    let parsed = arr
        .iter()
        .filter_map(|ev| serde_json::from_value::<nostr::Event>(ev.clone()).ok())
        .collect();
    Ok((parsed, arr.len()))
}

/// Fetch definitions of `kind` authored by the CLI identity, up to
/// [`LIST_LIMIT`] events.
///
/// The relay retains one event per coordinate, but a response may still carry
/// several events for one d-tag across reconnects, so this keeps the NIP-33
/// winner per d-tag: greatest `created_at`, ties broken by lowest event id.
///
/// Not paginated. An identity at the cap gets a truncated listing, which would
/// also make `teams create` report a published persona as missing, so a hit on
/// the cap warns rather than silently truncating.
pub async fn list_owned(client: &BuzzClient, kind: u32) -> Result<Vec<nostr::Event>, CliError> {
    let filter = serde_json::json!({
        "kinds": [kind],
        "authors": [client.keys().public_key().to_hex()],
        "limit": LIST_LIMIT,
    });
    let (events, returned) = parse_events(&client.query(&filter).await?)?;
    if returned >= LIST_LIMIT {
        eprintln!(
            "warning: hit the {LIST_LIMIT}-event listing cap for kind {kind}; \
             results may be incomplete"
        );
    }

    let mut heads: Vec<nostr::Event> = Vec::new();
    for event in events {
        let Some(d_tag) = event_d_tag(&event).map(str::to_owned) else {
            continue; // not addressable — cannot belong to any coordinate
        };
        match heads
            .iter()
            .position(|e| event_d_tag(e) == Some(d_tag.as_str()))
        {
            Some(i) if supersedes(&event, &heads[i]) => heads[i] = event,
            Some(_) => {}
            None => heads.push(event),
        }
    }
    heads.sort_by(|a, b| event_d_tag(a).cmp(&event_d_tag(b)));
    Ok(heads)
}

/// NIP-33 head selection: greater `created_at` wins, ties broken by lower id.
fn supersedes(candidate: &nostr::Event, incumbent: &nostr::Event) -> bool {
    match candidate.created_at.cmp(&incumbent.created_at) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => candidate.id < incumbent.id,
    }
}

/// Fetch the retained head for one `(kind, owner, d_tag)` coordinate.
///
/// A head that exists but does not deserialize is an error, not `None`:
/// callers read `None` as "coordinate is free" and would then stamp a write at
/// a bare `now`, which can lose the NIP-33 tie-break to the head they could not
/// read — or, for a delete, be silently discarded by the relay.
pub async fn fetch_head(
    client: &BuzzClient,
    kind: u32,
    d_tag: &str,
) -> Result<Option<nostr::Event>, CliError> {
    let filter = serde_json::json!({
        "kinds": [kind],
        "authors": [client.keys().public_key().to_hex()],
        "#d": [d_tag],
        "limit": 16,
    });
    let (events, returned) = parse_events(&client.query(&filter).await?)?;
    if events.is_empty() && returned > 0 {
        return Err(CliError::Other(format!(
            "relay holds {returned} unreadable event(s) at kind {kind} d-tag \
             '{d_tag}'; refusing to write over a head this CLI cannot parse"
        )));
    }
    Ok(events
        .into_iter()
        .fold(None, |head: Option<nostr::Event>, e| match head {
            Some(h) if !supersedes(&e, &h) => Some(h),
            _ => Some(e),
        }))
}

/// Stamp for a write at `now` over `prior_head`.
///
/// Passing `None` for a coordinate that does have a head is the silent failure
/// this exists to make testable: the write lands at a bare `now`, which a head
/// already bumped into the future outranks — and for a kind:5 tombstone the
/// relay accepts the miss and reports OK.
fn write_created_at(now: u64, prior_head: Option<&nostr::Event>) -> u64 {
    buzz_core::engram::monotonic_created_at(now, prior_head.map(|e| e.created_at.as_secs()))
}

/// Sign `builder` at the NIP-AP monotonic `created_at` for this coordinate and
/// submit it.
///
/// `prior_head` is the coordinate's current head, if any. NIP-33 keeps the
/// greatest `created_at` and breaks ties by lowest event id, so a same-second
/// rewrite can otherwise lose to the event it was meant to replace; the bump
/// past the head makes the write win regardless of clock skew.
///
/// Returns the signed event and the relay's normalized write response, which
/// callers emit on stdout per the buzz-cli write contract.
pub async fn publish_definition(
    client: &BuzzClient,
    builder: EventBuilder,
    prior_head: Option<&nostr::Event>,
) -> Result<(nostr::Event, serde_json::Value), CliError> {
    let created_at = write_created_at(now_secs(), prior_head);
    let event = builder
        .custom_created_at(nostr::Timestamp::from(created_at))
        .sign_with_keys(client.keys())
        .map_err(|e| CliError::Other(format!("failed to sign event: {e}")))?;

    let raw = client.submit_event(event.clone()).await?;
    let normalized = super::parse_write_response(
        &raw,
        "relay reported the write as duplicate / dominated by a newer head",
    )?;
    let response = serde_json::from_str(&normalized)
        .map_err(|e| CliError::Other(format!("relay response is not JSON: {e}")))?;
    Ok((event, response))
}

/// Emit a write response on stdout with the entity's own id folded in.
///
/// The buzz-cli contract is that writes return `{event_id, accepted, message}`
/// and creates add the entity id, so agent callers can consume the result.
pub fn print_write_response(mut response: serde_json::Value, key: &str, value: &str) {
    if let Some(obj) = response.as_object_mut() {
        obj.insert(
            key.to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }
    println!("{response}");
}

/// Read a definition body from a file, or stdin when `path` is `-`, rejecting
/// an empty one.
///
/// An empty prompt or instruction file almost always means an upstream step
/// produced nothing, and publishing it would silently blank the definition.
pub fn read_body_file(path: &str, what: &str) -> Result<String, CliError> {
    let body = crate::validate::read_file_or_stdin(path)?;
    if body.trim().is_empty() {
        return Err(CliError::Usage(format!(
            "{what} file '{path}' is empty — refusing to publish a blank {what}"
        )));
    }
    Ok(body)
}

/// The snapshot schema version this CLI knows how to read.
///
/// Matches Buzz Desktop's own snapshot validators, which reject anything else.
pub const SNAPSHOT_VERSION: u64 = 1;

/// Largest agent snapshot this CLI reads, mirroring `MAX_SNAPSHOT_JSON_BYTES`
/// in Buzz Desktop's persona import.
pub const MAX_AGENT_SNAPSHOT_BYTES: usize = 5 * 1024 * 1024;

/// Largest team snapshot this CLI reads, mirroring `MAX_TEAM_SNAPSHOT_JSON_BYTES`
/// in Buzz Desktop's team import. Larger than the agent cap because a team
/// carries a snapshot per member.
pub const MAX_TEAM_SNAPSHOT_BYTES: usize = 25 * 1024 * 1024;

/// Load and parse a Buzz Desktop snapshot export (`.agent.json` /
/// `.team.json`), checking the `format` and `version` envelope before the caller
/// reads its payload. Refuses a file past `max_bytes`.
pub fn read_snapshot(
    path: &str,
    expected_format: &str,
    max_bytes: usize,
) -> Result<serde_json::Value, CliError> {
    use std::io::Read;

    let file = std::fs::File::open(path)
        .map_err(|e| CliError::Usage(format!("cannot read '{path}': {e}")))?;
    // Read one byte past the cap rather than trusting metadata: the length a
    // fifo or a file growing mid-read reports need not be what arrives.
    let mut raw = String::new();
    file.take(max_bytes as u64 + 1)
        .read_to_string(&mut raw)
        .map_err(|e| CliError::Usage(format!("cannot read '{path}': {e}")))?;
    if raw.len() > max_bytes {
        return Err(CliError::Usage(format!(
            "'{path}' is larger than the {max_bytes}-byte snapshot limit, \
             which Buzz Desktop also refuses to import"
        )));
    }
    let value: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Usage(format!("'{path}' is not valid JSON: {e}")))?;
    let format = value.get("format").and_then(|v| v.as_str()).unwrap_or("");
    if format != expected_format {
        return Err(CliError::Usage(format!(
            "'{path}' has format '{format}', expected '{expected_format}'"
        )));
    }
    // Field meanings are version-scoped, so reading a future export under v1
    // semantics would misinterpret it rather than fail.
    match value.get("version").and_then(serde_json::Value::as_u64) {
        Some(SNAPSHOT_VERSION) => {}
        Some(v) => {
            return Err(CliError::Usage(format!(
                "'{path}' is snapshot version {v}, but this CLI reads version \
                 {SNAPSHOT_VERSION} — upgrade buzz"
            )));
        }
        None => {
            return Err(CliError::Usage(format!(
                "'{path}' has no numeric `version` field"
            )));
        }
    }
    Ok(value)
}

/// Print an event as a sig-stripped one-element JSON array.
///
/// The buzz-cli contract is that reads emit sig-stripped arrays, so a `get`
/// stays parseable by the same consumer that reads a `list`.
pub fn print_event_json(event: &nostr::Event) -> Result<(), CliError> {
    let value = serde_json::from_str(&event.as_json())
        .map_err(|e| CliError::Other(format!("failed to re-encode event: {e}")))?;
    println!("{}", crate::client::normalize_events(&[value]));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{Keys, Kind, Tag};

    fn event_at(d_tag: &str, created_at: u64) -> nostr::Event {
        EventBuilder::new(Kind::Custom(30175), "{}")
            .tags(vec![Tag::parse(["d", d_tag]).unwrap()])
            .custom_created_at(nostr::Timestamp::from(created_at))
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn later_created_at_supersedes() {
        let old = event_at("x", 100);
        let new = event_at("x", 200);
        assert!(supersedes(&new, &old));
        assert!(!supersedes(&old, &new));
    }

    /// NIP-33 breaks a `created_at` tie by lowest event id, so a rewrite at the
    /// same second is not guaranteed to win — the reason writes bump past the
    /// head instead of relying on the tiebreak.
    #[test]
    fn created_at_tie_breaks_on_lower_id() {
        let (a, b) = (event_at("x", 100), event_at("x", 100));
        let (lower, higher) = if a.id < b.id { (a, b) } else { (b, a) };
        assert!(supersedes(&lower, &higher));
        assert!(!supersedes(&higher, &lower));
    }

    /// A rewrite in the same second bumps the head past the wall clock, so a
    /// later write — including a delete — must be stamped from the head, not
    /// from `now`, or the relay keeps the event it was meant to replace.
    #[test]
    fn a_write_is_stamped_past_the_head_it_replaces() {
        assert_eq!(write_created_at(100, None), 100);
        assert_eq!(write_created_at(100, Some(&event_at("x", 100))), 101);
        assert_eq!(
            write_created_at(100, Some(&event_at("x", 200))),
            201,
            "a head ahead of the clock must still be outranked"
        );
    }

    #[test]
    fn read_body_file_rejects_blank_input() {
        let dir = std::env::temp_dir().join(format!("buzz-defs-{}", now_secs()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("blank.md");
        std::fs::write(&path, "   \n\t\n").unwrap();
        let err = read_body_file(path.to_str().unwrap(), "prompt").unwrap_err();
        assert!(err.to_string().contains("refusing to publish"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_snapshot_rejects_the_wrong_envelope() {
        let dir = std::env::temp_dir().join(format!("buzz-defs-snap-{}", now_secs()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wrong.json");
        std::fs::write(&path, r#"{"format":"buzz-team-snapshot","version":1}"#).unwrap();
        let err = read_snapshot(
            path.to_str().unwrap(),
            "buzz-agent-snapshot",
            MAX_AGENT_SNAPSHOT_BYTES,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("expected 'buzz-agent-snapshot'"),
            "{err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
