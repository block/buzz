//! NIP-AD agent dispositions (kind:44300) — emit and audit how an agent
//! resolved a human→agent request. See `docs/nips/NIP-AD.md`.
//!
//! All verification and lifecycle logic lives in
//! [`buzz_core::disposition`], not here. This module only fetches events,
//! adapts them into that verifier's input shape, and formats its output.
//! An earlier version derived state locally, and it drifted from the
//! desktop's independent derivation — different candidate sets, different
//! binding rules — which is exactly the failure a shared verifier prevents.

use buzz_core::disposition::{
    account, classify_request, derive_obligation, Accounting, Coverage, EffectiveOutcome,
    EventView, HistoryWarning, InvalidRequest, RequestClass, UnsupportedRequest, REQUEST_KINDS,
};
use buzz_core::kind::KIND_AGENT_DISPOSITION;
use nostr::JsonUtil;

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{parse_event_id, parse_uuid, validate_hex64};

/// Owned event fields, so [`EventView`]s can borrow from a stable buffer.
/// The verifier is deliberately borrow-based (it runs in the relay, the
/// CLI, and an auditor without imposing an event type), so callers own the
/// storage.
struct OwnedEvent {
    id: String,
    pubkey: String,
    kind: u16,
    created_at: i64,
    content: String,
    tags: Vec<Vec<String>>,
}

impl OwnedEvent {
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            id: value.get("id")?.as_str()?.to_string(),
            pubkey: value.get("pubkey")?.as_str()?.to_string(),
            kind: value.get("kind").and_then(|v| v.as_u64()).unwrap_or(0) as u16,
            created_at: value
                .get("created_at")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
            content: value
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            tags: value
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|rows| {
                    rows.iter()
                        .filter_map(|row| {
                            Some(
                                row.as_array()?
                                    .iter()
                                    .filter_map(|c| c.as_str().map(String::from))
                                    .collect(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    fn view(&self) -> EventView<'_> {
        EventView {
            id: &self.id,
            pubkey: &self.pubkey,
            kind: self.kind,
            created_at: self.created_at,
            content: &self.content,
            tags: &self.tags,
        }
    }
}

fn parse_events(raw: &str, what: &str) -> Result<Vec<OwnedEvent>, CliError> {
    Ok(parse_verified_events(raw, what)?.0)
}

/// Parse relay JSON into owned events, **verifying each event's id and
/// signature** and dropping any that fail. Returns the survivors and the
/// number rejected.
///
/// Without this, `buzz dispositions list` was a semantic verifier that
/// trusted the relay: it applied every NIP-AD binding rule perfectly to
/// events it had not checked were authentic. The relay does verify at ingest,
/// so this changes nothing against an honest relay — which is precisely why
/// it was easy to leave out, and precisely why it matters. An auditor whose
/// guarantees dissolve if the relay is lying is not an independent auditor,
/// and this tool's whole purpose is to be one.
fn parse_verified_events(raw: &str, what: &str) -> Result<(Vec<OwnedEvent>, usize), CliError> {
    let values: Vec<serde_json::Value> = serde_json::from_str(raw)
        .map_err(|e| CliError::Other(format!("failed to parse {what}: {e}")))?;

    let mut events = Vec::with_capacity(values.len());
    let mut rejected = 0usize;
    for value in &values {
        // Reads are sig-stripped on some CLI paths; an event with no `sig`
        // cannot be verified and must not be silently trusted either.
        let verified = serde_json::to_string(value)
            .ok()
            .and_then(|json| nostr::Event::from_json(&json).ok())
            .is_some_and(|event| event.verify().is_ok());
        if !verified {
            rejected += 1;
            continue;
        }
        if let Some(event) = OwnedEvent::from_json(value) {
            events.push(event);
        } else {
            rejected += 1;
        }
    }
    Ok((events, rejected))
}

/// Emit a disposition for a request this agent handled.
///
/// Only `request` and `disposition` are required — the requester's pubkey
/// (`p` tag) and the channel (`h` tag) are both derived by fetching the
/// request event itself, rather than asking the caller to repeat data it
/// would otherwise have to get right independently. This removes a whole
/// class of mistake (a mistyped requester pubkey, or a channel that doesn't
/// actually match the request) rather than merely validating against it.
///
/// The request is checked against the shared verifier first, so the CLI
/// refuses to answer a request that no reader could ever bind the answer
/// to — publishing an unbindable disposition would look like a successful
/// write while leaving the request permanently unanswered.
///
/// **This is the path a managed agent uses to settle its own work.** The
/// harness structurally cannot emit `completed` (it observes that a turn
/// ended, never that the task was done), so `completed` reaches the ledger
/// only from here, signed by the agent that was actually asked.
pub async fn cmd_emit(
    client: &BuzzClient,
    request_event_id: &str,
    disposition: &str,
    reason: &str,
) -> Result<(), CliError> {
    validate_hex64(request_event_id)?;
    let request_eid = parse_event_id(request_event_id)?;

    let filter = serde_json::json!({ "ids": [request_event_id] });
    let raw = client.query(&filter).await?;
    let events = parse_events(&raw, "request lookup")?;
    let request = events
        .first()
        .ok_or_else(|| CliError::NotFound(format!("request event {request_event_id} not found")))?;
    let view = request.view();

    let obligation = match classify_request(&view) {
        RequestClass::Valid(obligation) => obligation,
        RequestClass::NotRequest => {
            return Err(CliError::Usage(format!(
            "event {request_event_id} is not a NIP-AD request (no [\"t\",\"request\"] marker) — \
                 a disposition against it would never bind"
        )))
        }
        RequestClass::Invalid(reason) => {
            return Err(CliError::Usage(format!(
                "request {request_event_id} is not a valid NIP-AD v1 obligation ({}) — \
                 a disposition against it would never bind. See docs/nips/NIP-AD.md.",
                describe_invalid(reason)
            )))
        }
        RequestClass::Unsupported(reason) => {
            return Err(CliError::Usage(format!(
                "request {request_event_id} is not representable in NIP-AD v1 ({}) — \
                 a disposition against it would never bind. See docs/nips/NIP-AD.md.",
                describe_unsupported(reason)
            )))
        }
    };

    // Only the obligation's target agent can discharge it. Without this the
    // CLI happily signed and published a disposition from any identity and
    // reported success, while every consumer ignored it — a write that looks
    // like it worked and changes nothing is worse than a rejection.
    let signer = client.keys().public_key().to_hex();
    if signer != obligation.target_agent_pubkey {
        return Err(CliError::Usage(format!(
            "this identity ({signer}) is not the agent request {request_event_id} was \
             addressed to ({}) — only the target agent can discharge an obligation, so \
             a disposition signed here would be stored and then ignored by every reader",
            obligation.target_agent_pubkey
        )));
    }

    let channel_uuid = parse_uuid(&obligation.channel_id)?;
    let builder = buzz_sdk::build_agent_disposition(
        channel_uuid,
        request_eid,
        &obligation.requester_pubkey,
        disposition,
        reason,
    )
    .map_err(|e| CliError::Other(format!("build_agent_disposition failed: {e}")))?;
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

fn describe_invalid(reason: InvalidRequest) -> &'static str {
    match reason {
        InvalidRequest::MissingAgentTarget => {
            "it names no agent target, so nothing was asked of anyone"
        }
        InvalidRequest::MalformedAgentTarget => {
            "its agent target is not canonical 64-char lowercase hex"
        }
        InvalidRequest::MissingChannel => "it has no channel (h tag)",
        InvalidRequest::MultipleChannels => {
            "it carries more than one channel (h tag), so its scope is ambiguous"
        }
        InvalidRequest::TargetNotMentioned => {
            "its agent target is not also p-mentioned, so the request never reached it"
        }
        InvalidRequest::DuplicateAgentTarget => {
            "it repeats the same agent target, which is not a canonical v1 request"
        }
        InvalidRequest::UnsupportedKind => "its event kind is not one v1 accepts requests on",
    }
}

fn describe_unsupported(reason: UnsupportedRequest) -> &'static str {
    match reason {
        UnsupportedRequest::MultipleAgentTargets => {
            "it names several agent targets, and v1 cannot say whether either target \
             may answer or both must — a future revision may add per-agent obligations"
        }
    }
}

fn invalid_reason_slug(reason: InvalidRequest) -> &'static str {
    match reason {
        InvalidRequest::MissingAgentTarget => "missing_agent_target",
        InvalidRequest::MalformedAgentTarget => "malformed_agent_target",
        InvalidRequest::MissingChannel => "missing_channel",
        InvalidRequest::MultipleChannels => "multiple_channels",
        InvalidRequest::TargetNotMentioned => "target_not_mentioned",
        InvalidRequest::DuplicateAgentTarget => "duplicate_agent_target",
        InvalidRequest::UnsupportedKind => "unsupported_kind",
    }
}

fn unsupported_reason_slug(reason: UnsupportedRequest) -> &'static str {
    match reason {
        UnsupportedRequest::MultipleAgentTargets => "multiple_agent_targets",
    }
}

fn warning_slug(warning: HistoryWarning) -> &'static str {
    match warning {
        HistoryWarning::DuplicateTerminal => "duplicate_terminal",
        HistoryWarning::OrderedAfterTerminal => "ordered_after_terminal",
    }
}

/// List a channel's dispositions with obligation accounting.
///
/// `state`, when given, narrows the returned rows — but as a CLIENT-side
/// filter applied after fetching by `#h`, never as a relay query key.
/// `#disposition` is not a valid NIP-01 single-letter tag filter, and the
/// underlying nostr crate's `Filter.generic_tags` silently drops an
/// unrecognized multi-char key rather than erroring, so sending it would
/// return every state while looking like a filtered query. See NIP-AD.md's
/// "Not a query filter".
///
/// The `state` filter never changes the accounting totals — those always
/// come from the full unfiltered set.
pub async fn cmd_list(
    client: &BuzzClient,
    channel_id: &str,
    state: Option<&str>,
) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;
    let channel_str = channel_uuid.to_string();

    let raw = client
        .query(&serde_json::json!({
            "kinds": [KIND_AGENT_DISPOSITION],
            "#h": [channel_str],
        }))
        .await?;
    let (dispositions, rejected_dispositions) = parse_verified_events(&raw, "dispositions query")?;

    // The request universe comes from the shared `REQUEST_KINDS`, not a
    // locally chosen kind list — two consumers querying different kind sets
    // produce different accounting for one channel and both look correct.
    let raw = client
        .query(&serde_json::json!({
            "kinds": REQUEST_KINDS,
            "#h": [channel_str],
            "#t": ["request"],
        }))
        .await?;
    let (requests, rejected_requests) = parse_verified_events(&raw, "marked-requests query")?;

    let request_views: Vec<EventView<'_>> = requests.iter().map(OwnedEvent::view).collect();
    let disposition_views: Vec<EventView<'_>> = dispositions.iter().map(OwnedEvent::view).collect();

    // Both sides of coverage are false: this is a single unpaginated query
    // pair with no completeness token, so it describes what the relay
    // returned, not provably the channel's whole history. Requests alone
    // would not be enough anyway — an unfetched later `refused` on page two
    // turns a "settled" obligation into a disputed one.
    let acc = account(&request_views, &disposition_views, Coverage::partial());

    // Per-request rows, filtered only for display.
    let mut rows = Vec::new();
    for request in &request_views {
        let RequestClass::Valid(obligation) = classify_request(request) else {
            continue;
        };
        let derived = derive_obligation(&obligation, &disposition_views);
        let (outcome_kind, current) = match derived.outcome {
            EffectiveOutcome::Unanswered => continue,
            EffectiveOutcome::Open(s) => ("open", Some(s)),
            EffectiveOutcome::Settled(s) => ("settled", Some(s)),
            EffectiveOutcome::Disputed => ("disputed", None),
        };
        if let Some(want) = state {
            if current.is_none_or(|c| c.as_str() != want) {
                continue;
            }
        }
        rows.push(serde_json::json!({
            "request_id": obligation.request_id,
            "target_agent": obligation.target_agent_pubkey,
            "outcome": outcome_kind,
            "disposition": current.map(|c| c.as_str()),
            // What arrived last, which is a different question from what is
            // true — a terminal claim absorbs later weaker observations.
            "latest_observation": derived.latest_observation.map(|s| s.as_str()),
            "reason": derived.reason,
            "warnings": derived.warnings.iter().map(|w| warning_slug(*w)).collect::<Vec<_>>(),
            "resolved": derived.is_resolved(),
        }));
    }

    println!(
        "{}",
        serde_json::to_string(&render(
            &acc,
            rows,
            &channel_str,
            rejected_requests + rejected_dispositions,
        ))
        .unwrap_or_default()
    );
    Ok(())
}

fn render(
    acc: &Accounting,
    rows: Vec<serde_json::Value>,
    channel: &str,
    unverifiable_events: usize,
) -> serde_json::Value {
    serde_json::json!({
        "channel": channel,
        // Events the relay returned that failed id/signature verification and
        // were excluded before any accounting. Non-zero means the relay
        // served something it should not have.
        "unverifiable_events": unverifiable_events,
        "marked_requests": acc.total(),
        // Settled: a terminal claim, absorbing.
        "settled": acc.settled,
        // Answered but not settled — `responded` or `errored`.
        "open": acc.open,
        // Nothing bound at all: a real gap.
        "unanswered": acc.unanswered,
        // Both terminal states claimed. No settled answer exists.
        "disputed": acc.disputed,
        // Malformed marked events. Client faults, NOT agent failures —
        // reporting them as unanswered would blame an agent for a gap no
        // agent could ever clear.
        "invalid_requests": acc.invalid_requests
            .iter()
            .map(|(id, reason)| serde_json::json!({
                "request_id": id,
                "reason": invalid_reason_slug(*reason),
            }))
            .collect::<Vec<_>>(),
        // Well-formed but not representable in v1. Nobody's fault; still
        // blocks a clean claim.
        "unsupported_requests": acc.unsupported_requests
            .iter()
            .map(|(id, reason)| serde_json::json!({
                "request_id": id,
                "reason": unsupported_reason_slug(*reason),
            }))
            .collect::<Vec<_>>(),
        // Stored dispositions that named an obligation but did not bind —
        // spoof attempts and misdirected writes, visible without being
        // allowed to affect any outcome.
        "rejected_claims": acc.rejected_claims
            .iter()
            .map(|c| serde_json::json!({
                "event_id": c.event_id,
                "referenced_request": c.referenced_request,
                "signer": c.signer,
                "failure": format!("{:?}", c.failure),
            }))
            .collect::<Vec<_>>(),
        // Whether the above covers the channel's whole history, on BOTH
        // sides. Always false here: one unpaginated query pair, no
        // completeness token.
        "coverage": {
            "requests_complete": acc.coverage.requests_complete,
            "dispositions_complete": acc.coverage.dispositions_complete,
        },
        // Only true with complete coverage AND nothing open, disputed,
        // invalid, or unsupported. The one field safe to render as "all good".
        "all_resolved": acc.all_resolved(),
        "dispositions": rows,
    })
}

pub async fn dispatch(cmd: crate::DispositionsCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::DispositionsCmd;
    match cmd {
        DispositionsCmd::Emit {
            request,
            disposition,
            reason,
        } => {
            cmd_emit(
                client,
                &request,
                &disposition,
                reason.as_deref().unwrap_or(""),
            )
            .await
        }
        DispositionsCmd::List { channel, state } => {
            cmd_list(client, &channel, state.as_deref()).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HUMAN: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const REQUESTER: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    const CHANNEL: &str = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";
    const REQ_A: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const REQ_B: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    fn request_json(id: &str, agent: Option<&str>) -> serde_json::Value {
        let mut tags = vec![
            serde_json::json!(["h", CHANNEL]),
            serde_json::json!(["t", "request"]),
            serde_json::json!(["p", HUMAN]),
        ];
        if let Some(agent) = agent {
            tags.push(serde_json::json!(["agent", agent]));
            tags.push(serde_json::json!(["p", agent]));
        }
        serde_json::json!({
            "id": id,
            "pubkey": REQUESTER,
            "kind": REQUEST_KINDS[0],
            "created_at": 100,
            "content": "@agent do the thing",
            "tags": tags,
        })
    }

    fn disposition_json(id: &str, request: &str, signer: &str, state: &str) -> serde_json::Value {
        disposition_json_at(id, request, signer, state, 200)
    }

    fn disposition_json_at(
        id: &str,
        request: &str,
        signer: &str,
        state: &str,
        created_at: i64,
    ) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "pubkey": signer,
            "kind": KIND_AGENT_DISPOSITION,
            "created_at": created_at,
            "content": serde_json::json!({"disposition": state, "reason": ""}).to_string(),
            "tags": [
                ["e", request],
                ["h", CHANNEL],
                ["p", REQUESTER],
                ["disposition", state],
            ],
        })
    }

    fn owned(values: &[serde_json::Value]) -> Vec<OwnedEvent> {
        values.iter().filter_map(OwnedEvent::from_json).collect()
    }

    #[test]
    fn an_unsigned_or_forged_event_is_dropped_before_any_accounting() {
        // The auditor's independence rests on this: every rule below is
        // applied only to events whose id and signature check out. An event
        // the relay served but nobody validly signed must not reach the
        // verifier at all.
        let forged = serde_json::json!([{
            "id": REQ_A,
            "pubkey": AGENT,
            "kind": REQUEST_KINDS[0],
            "created_at": 100,
            "content": "@agent do the thing",
            "tags": [["h", CHANNEL], ["t", "request"]],
            "sig": "00".repeat(64),
        }]);
        let (events, rejected) =
            parse_verified_events(&forged.to_string(), "test").expect("valid JSON");
        assert!(
            events.is_empty(),
            "a bad signature must not survive parsing"
        );
        assert_eq!(rejected, 1);
    }

    #[test]
    fn an_invalid_request_is_reported_as_invalid_not_unanswered() {
        // A targetless marked request can never be answered by anyone.
        // Counting it as a gap would blame an agent for a composer fault and
        // leave a gap that never clears.
        let reqs = owned(&[request_json(REQ_A, None)]);
        let views: Vec<EventView<'_>> = reqs.iter().map(OwnedEvent::view).collect();
        let acc = account(&views, &[], Coverage::complete());
        assert!(acc.unanswered.is_empty());
        assert_eq!(acc.invalid_requests.len(), 1);
        assert_eq!(
            invalid_reason_slug(acc.invalid_requests[0].1),
            "missing_agent_target"
        );
        assert!(!acc.all_resolved());
    }

    #[test]
    fn a_settled_obligation_is_not_reopened_by_a_later_error() {
        // Terminal absorption, through the CLI's own adapter: the agent
        // asserted `completed`, then a stray `errored` sorted after it. The
        // obligation stays settled and the stray write is a warning.
        let reqs = owned(&[request_json(REQ_A, Some(AGENT))]);
        let disps = owned(&[
            disposition_json_at("d1", REQ_A, AGENT, "completed", 200),
            disposition_json_at("d2", REQ_A, AGENT, "errored", 300),
        ]);
        let rv: Vec<EventView<'_>> = reqs.iter().map(OwnedEvent::view).collect();
        let dv: Vec<EventView<'_>> = disps.iter().map(OwnedEvent::view).collect();
        let acc = account(&rv, &dv, Coverage::complete());
        assert_eq!(acc.settled, vec![REQ_A.to_string()]);
        assert!(acc.open.is_empty());
        assert!(acc.all_resolved());
    }

    #[test]
    fn a_multi_target_request_is_unsupported_not_an_agent_gap() {
        let mut req = request_json(REQ_A, Some(AGENT));
        req["tags"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!(["agent", HUMAN]));
        req["tags"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!(["p", HUMAN]));
        let reqs = owned(&[req]);
        let views: Vec<EventView<'_>> = reqs.iter().map(OwnedEvent::view).collect();
        let acc = account(&views, &[], Coverage::complete());
        assert!(acc.unanswered.is_empty(), "never an agent gap");
        assert!(acc.invalid_requests.is_empty(), "not a malformed event");
        assert_eq!(acc.unsupported_requests.len(), 1);
        assert_eq!(
            unsupported_reason_slug(acc.unsupported_requests[0].1),
            "multiple_agent_targets"
        );
        assert!(!acc.all_resolved());
    }

    #[test]
    fn a_responded_request_is_open_not_resolved() {
        // The state that keeps the harness honest: the agent answered, but
        // nothing asserted the work was done.
        let reqs = owned(&[request_json(REQ_A, Some(AGENT))]);
        let disps = owned(&[disposition_json("d1", REQ_A, AGENT, "responded")]);
        let rv: Vec<EventView<'_>> = reqs.iter().map(OwnedEvent::view).collect();
        let dv: Vec<EventView<'_>> = disps.iter().map(OwnedEvent::view).collect();
        let acc = account(&rv, &dv, Coverage::complete());
        assert_eq!(acc.open, vec![REQ_A.to_string()]);
        assert!(acc.settled.is_empty());
        assert!(!acc.all_resolved());
    }

    #[test]
    fn a_disposition_from_a_merely_mentioned_human_leaves_the_request_unanswered() {
        let reqs = owned(&[request_json(REQ_A, Some(AGENT))]);
        let disps = owned(&[disposition_json("d1", REQ_A, HUMAN, "completed")]);
        let rv: Vec<EventView<'_>> = reqs.iter().map(OwnedEvent::view).collect();
        let dv: Vec<EventView<'_>> = disps.iter().map(OwnedEvent::view).collect();
        let acc = account(&rv, &dv, Coverage::complete());
        assert_eq!(acc.unanswered, vec![REQ_A.to_string()]);
        assert!(acc.settled.is_empty());
    }

    #[test]
    fn accounting_is_never_claimed_complete_from_one_unpaginated_query() {
        // `cmd_list` passes scope_complete=false. Even a perfectly clean
        // channel must not report `all_resolved`, because the query pair
        // carries no completeness guarantee.
        let reqs = owned(&[request_json(REQ_A, Some(AGENT))]);
        let disps = owned(&[disposition_json("d1", REQ_A, AGENT, "completed")]);
        let rv: Vec<EventView<'_>> = reqs.iter().map(OwnedEvent::view).collect();
        let dv: Vec<EventView<'_>> = disps.iter().map(OwnedEvent::view).collect();

        let partial = account(&rv, &dv, Coverage::partial());
        assert_eq!(partial.settled, vec![REQ_A.to_string()]);
        assert!(
            !partial.all_resolved(),
            "an unpaginated read must never claim channel-wide resolution"
        );
        assert!(account(&rv, &dv, Coverage::complete()).all_resolved());
    }

    #[test]
    fn rendered_output_separates_every_category() {
        let reqs = owned(&[request_json(REQ_A, Some(AGENT)), request_json(REQ_B, None)]);
        let disps = owned(&[disposition_json("d1", REQ_A, AGENT, "completed")]);
        let rv: Vec<EventView<'_>> = reqs.iter().map(OwnedEvent::view).collect();
        let dv: Vec<EventView<'_>> = disps.iter().map(OwnedEvent::view).collect();
        let acc = account(&rv, &dv, Coverage::complete());
        let out = render(&acc, vec![], CHANNEL, 0);

        assert_eq!(out["marked_requests"], 2);
        assert_eq!(out["settled"][0], REQ_A);
        assert_eq!(out["invalid_requests"][0]["request_id"], REQ_B);
        assert_eq!(out["invalid_requests"][0]["reason"], "missing_agent_target");
        assert_eq!(out["all_resolved"], false);
        // The legacy `all_answered` field is gone: it conflated "has any
        // disposition" with "settled", so a channel of nothing but failures
        // reported true.
        assert!(out.get("all_answered").is_none());
    }
}
