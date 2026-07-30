//! Reduces signed journal handoff events into a cryptographically authorized,
//! causally linked lifecycle state.
//!
//! Mutable labels such as `agent`, `runner`, and `verifier` are presentation
//! only. Authority comes from event authorship, verified NIP-OA owner
//! attestations, the opener's `p`-tagged claimant set, and explicit event links.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read};

use anyhow::{bail, Context};
use buzz_core::verification::verify_event;
use nostr::{Event, PublicKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const KIND_TEXT_NOTE: u16 = 1;
const HANDOFF_OPEN: &str = "handoff:open";
const HANDOFF_CLAIM: &str = "handoff:claim";
const HANDOFF_RETURN: &str = "handoff:return";
const HANDOFF_CLOSE: &str = "handoff:close";
const HANDOFF_ACK_INVALID: &str = "handoff:ack-invalid";
const MAX_INVALID_ACK_TARGETS: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum LifecycleState {
    Open,
    Claimed,
    Conflict,
    Returned,
    Closed,
    Invalid,
    AcknowledgedInvalid,
}

#[derive(Debug, Serialize)]
struct IgnoredEvent {
    id: String,
    reason: String,
}

#[derive(Debug, Serialize)]
struct HandoffState {
    open_id: String,
    title: String,
    context: String,
    opener_pubkey: String,
    opener_owner_pubkey: String,
    allowed_claimants: Vec<String>,
    created_at: u64,
    updated_at: u64,
    state: LifecycleState,
    claim_id: Option<String>,
    claimant_pubkey: Option<String>,
    claim_created_at: Option<u64>,
    return_id: Option<String>,
    return_created_at: Option<u64>,
    close_id: Option<String>,
    close_created_at: Option<u64>,
    acknowledgment_id: Option<String>,
    acknowledgment_created_at: Option<u64>,
    conflicting_claims: Vec<String>,
    ignored: Vec<IgnoredEvent>,
}

#[derive(Debug)]
struct OpenAuthority {
    id: String,
    title: String,
    context: String,
    opener_pubkey: PublicKey,
    opener_owner_pubkey: PublicKey,
    allowed_claimants: BTreeSet<PublicKey>,
    created_at: u64,
}

#[derive(Clone, Copy, Debug)]
struct AcceptedEvent<'a> {
    event: &'a Event,
}

#[derive(Debug)]
struct InvalidOpenAuthority {
    id: String,
    context: String,
    opener_owner_pubkey: PublicKey,
    created_at: u64,
}

#[derive(Debug, Default)]
struct InvalidAcknowledgments<'a> {
    accepted_by_open: BTreeMap<String, Vec<AcceptedEvent<'a>>>,
    ignored_by_open: BTreeMap<String, Vec<IgnoredEvent>>,
}

#[derive(Debug)]
struct ValidatedInvalidAck<'a> {
    event: &'a Event,
    open_ids: BTreeSet<String>,
}

#[derive(Debug, Deserialize)]
struct OwnerAttestationRequest {
    signer_pubkey: String,
    auth_tag: Vec<String>,
    kind: u16,
    created_at: u64,
}

fn main() -> anyhow::Result<()> {
    let mut selected_open = None;
    let mut verify_owner_attestation = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--open" => {
                selected_open = Some(
                    args.next()
                        .context("--open requires a 64-character event id")?,
                );
            }
            "--verify-owner-attestation" => verify_owner_attestation = true,
            "-h" | "--help" => {
                println!(
                    "buzz-handoff-state [--open EVENT_ID]\n\
                     buzz-handoff-state --verify-owner-attestation\n\
                     Reduces a JSON array of signed Nostr events, or verifies an\n\
                     owner-attestation request, from stdin."
                );
                return Ok(());
            }
            other => bail!("unknown argument: {other}"),
        }
    }
    if verify_owner_attestation && selected_open.is_some() {
        bail!("--verify-owner-attestation cannot be combined with --open");
    }
    if let Some(open_id) = selected_open.as_ref() {
        if !is_lower_hex(open_id, 64) {
            bail!("--open requires a lowercase 64-character event id");
        }
    }

    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .context("could not read event JSON from stdin")?;
    if verify_owner_attestation {
        let request: OwnerAttestationRequest = serde_json::from_str(&input)
            .context("stdin must be a JSON owner-attestation request")?;
        let owner = verify_owner_attestation_request(&request).map_err(anyhow::Error::msg)?;
        println!("{}", owner.to_hex());
        return Ok(());
    }
    let events: Vec<Event> =
        serde_json::from_str(&input).context("stdin must be a JSON array of Nostr events")?;
    let invalid_acknowledgments = build_invalid_acknowledgments(&events);

    let mut states = Vec::new();
    for event in events
        .iter()
        .filter(|event| lifecycle(event) == Some(HANDOFF_OPEN))
    {
        let event_id = event.id.to_hex();
        if selected_open
            .as_ref()
            .is_some_and(|selected| selected != &event_id)
        {
            continue;
        }
        states.push(reduce_handoff_with_invalid_acknowledgments(
            event,
            &events,
            &invalid_acknowledgments,
        ));
    }
    states.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.open_id.cmp(&right.open_id))
    });

    if let Some(open_id) = selected_open {
        let state = states
            .into_iter()
            .next()
            .with_context(|| format!("handoff open event {open_id} was not found"))?;
        println!("{}", serde_json::to_string(&state)?);
    } else {
        println!("{}", serde_json::to_string(&states)?);
    }
    Ok(())
}

fn reduce_handoff_with_invalid_acknowledgments(
    open: &Event,
    events: &[Event],
    invalid_acknowledgments: &InvalidAcknowledgments<'_>,
) -> HandoffState {
    let open_id = open.id.to_hex();
    let mut ignored = Vec::new();
    let authority = match validate_open(open) {
        Ok(authority) => authority,
        Err(reason) => {
            let mut ignored = vec![IgnoredEvent {
                id: open.id.to_hex(),
                reason,
            }];
            let invalid_authority = invalid_open_authority(open).ok();
            if let Some(rejected) = invalid_acknowledgments.ignored_by_open.get(&open_id) {
                ignored.extend(rejected.iter().map(|event| IgnoredEvent {
                    id: event.id.clone(),
                    reason: event.reason.clone(),
                }));
            }
            let acknowledgments = invalid_acknowledgments
                .accepted_by_open
                .get(&open_id)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let acknowledgment = acknowledgments.first().map(|accepted| accepted.event);
            for duplicate in acknowledgments.iter().skip(1) {
                ignored.push(ignored_event(
                    duplicate.event,
                    "duplicate acknowledgment for invalid handoff",
                ));
            }
            return HandoffState {
                open_id,
                title: open_title(open),
                context: tag_values(open, "h").first().cloned().unwrap_or_default(),
                opener_pubkey: open.pubkey.to_hex(),
                opener_owner_pubkey: invalid_authority
                    .map(|authority| authority.opener_owner_pubkey.to_hex())
                    .unwrap_or_default(),
                allowed_claimants: Vec::new(),
                created_at: open.created_at.as_secs(),
                updated_at: acknowledgment.map_or(open.created_at.as_secs(), |event| {
                    event.created_at.as_secs()
                }),
                state: if acknowledgment.is_some() {
                    LifecycleState::AcknowledgedInvalid
                } else {
                    LifecycleState::Invalid
                },
                claim_id: None,
                claimant_pubkey: None,
                claim_created_at: None,
                return_id: None,
                return_created_at: None,
                close_id: None,
                close_created_at: None,
                acknowledgment_id: acknowledgment.map(|event| event.id.to_hex()),
                acknowledgment_created_at: acknowledgment.map(|event| event.created_at.as_secs()),
                conflicting_claims: Vec::new(),
                ignored,
            };
        }
    };

    let mut claims_by_author: BTreeMap<PublicKey, Vec<AcceptedEvent<'_>>> = BTreeMap::new();
    for event in events
        .iter()
        .filter(|event| lifecycle(event) == Some(HANDOFF_CLAIM) && references(event, &authority.id))
    {
        match validate_claim(event, &authority) {
            Ok(accepted) => claims_by_author
                .entry(event.pubkey)
                .or_default()
                .push(accepted),
            Err(reason) => ignored.push(ignored_event(event, reason)),
        }
    }
    for claims in claims_by_author.values_mut() {
        claims.sort_by(event_order);
    }

    if claims_by_author.len() > 1 {
        let mut conflicting_claims = Vec::new();
        for claims in claims_by_author.values() {
            if let Some(claim) = claims.first() {
                conflicting_claims.push(claim.event.id.to_hex());
            }
        }
        conflicting_claims.sort();
        return state_from_authority(
            authority,
            LifecycleState::Conflict,
            None,
            None,
            None,
            conflicting_claims,
            ignored,
        );
    }

    let Some(mut claims) = claims_by_author.into_values().next() else {
        return state_from_authority(
            authority,
            LifecycleState::Open,
            None,
            None,
            None,
            Vec::new(),
            ignored,
        );
    };
    let claim = claims.remove(0);
    for duplicate in claims {
        ignored.push(ignored_event(
            duplicate.event,
            "duplicate claim by the accepted claimant",
        ));
    }

    let mut returns = Vec::new();
    for event in events.iter().filter(|event| {
        lifecycle(event) == Some(HANDOFF_RETURN) && references(event, &authority.id)
    }) {
        match validate_return(event, &authority, claim.event) {
            Ok(accepted) => returns.push(accepted),
            Err(reason) => ignored.push(ignored_event(event, reason)),
        }
    }
    returns.sort_by(event_order);
    let Some(returned) = returns.pop() else {
        return state_from_authority(
            authority,
            LifecycleState::Claimed,
            Some(claim.event),
            None,
            None,
            Vec::new(),
            ignored,
        );
    };

    let mut closes = Vec::new();
    for event in events
        .iter()
        .filter(|event| lifecycle(event) == Some(HANDOFF_CLOSE) && references(event, &authority.id))
    {
        match validate_close(event, &authority, returned.event) {
            Ok(accepted) => closes.push(accepted),
            Err(reason) => ignored.push(ignored_event(event, reason)),
        }
    }
    closes.sort_by(event_order);
    let close = closes.first().map(|accepted| accepted.event);
    let state = if close.is_some() {
        LifecycleState::Closed
    } else {
        LifecycleState::Returned
    };
    state_from_authority(
        authority,
        state,
        Some(claim.event),
        Some(returned.event),
        close,
        Vec::new(),
        ignored,
    )
}

#[cfg(test)]
fn reduce_handoff(open: &Event, events: &[Event]) -> HandoffState {
    let invalid_acknowledgments = build_invalid_acknowledgments(events);
    reduce_handoff_with_invalid_acknowledgments(open, events, &invalid_acknowledgments)
}

fn validate_open(event: &Event) -> Result<OpenAuthority, String> {
    validate_envelope(event, HANDOFF_OPEN)?;
    let context = exactly_one_tag(event, "h")?;
    let content = content_object(event)?;
    let title = nonempty_string(&content, "title")?.to_string();
    let base_commit = nonempty_string(&content, "base_commit")?;
    if !is_lower_hex(base_commit, 40) {
        return Err("base_commit must be a canonical lowercase 40-character object id".into());
    }
    let allowed_claimants: BTreeSet<PublicKey> = tag_values(event, "p")
        .into_iter()
        .map(|value| {
            if !is_lower_hex(&value, 64) {
                return Err("p tags must contain lowercase 64-character pubkeys".to_string());
            }
            PublicKey::from_hex(&value)
                .map_err(|_| "p tags must contain lowercase 64-character pubkeys".to_string())
        })
        .collect::<Result<_, _>>()?;
    if allowed_claimants.is_empty() {
        return Err("open event must p-tag at least one allowed claimant".into());
    }
    if let Some(target_pubkey) = content
        .get("target")
        .and_then(Value::as_object)
        .and_then(|target| target.get("pubkey"))
        .and_then(Value::as_str)
    {
        if !is_lower_hex(target_pubkey, 64) {
            return Err("target.pubkey is not a lowercase 64-character public key".into());
        }
        let target = PublicKey::from_hex(target_pubkey)
            .map_err(|_| "target.pubkey is not a valid public key".to_string())?;
        if !allowed_claimants.contains(&target) {
            return Err("target.pubkey must also appear in a p tag".into());
        }
    }
    let owner = event_owner(event)?;
    Ok(OpenAuthority {
        id: event.id.to_hex(),
        title,
        context,
        opener_pubkey: event.pubkey,
        opener_owner_pubkey: owner,
        allowed_claimants,
        created_at: event.created_at.as_secs(),
    })
}

fn invalid_open_authority(event: &Event) -> Result<InvalidOpenAuthority, String> {
    validate_envelope(event, HANDOFF_OPEN)?;
    Ok(InvalidOpenAuthority {
        id: event.id.to_hex(),
        context: exactly_one_tag(event, "h")?,
        opener_owner_pubkey: event_owner(event)?,
        created_at: event.created_at.as_secs(),
    })
}

fn build_invalid_acknowledgments(events: &[Event]) -> InvalidAcknowledgments<'_> {
    let invalid_opens = events
        .iter()
        .filter(|event| lifecycle(event) == Some(HANDOFF_OPEN))
        .filter(|event| validate_open(event).is_err())
        .filter_map(|event| {
            invalid_open_authority(event)
                .ok()
                .map(|authority| (authority.id.clone(), authority))
        })
        .collect::<BTreeMap<_, _>>();
    let mut acknowledgments = InvalidAcknowledgments::default();

    for event in events
        .iter()
        .filter(|event| lifecycle(event) == Some(HANDOFF_ACK_INVALID))
    {
        match validate_invalid_ack(event, &invalid_opens) {
            Ok(validated) => {
                for open_id in validated.open_ids {
                    acknowledgments
                        .accepted_by_open
                        .entry(open_id)
                        .or_default()
                        .push(AcceptedEvent {
                            event: validated.event,
                        });
                }
            }
            Err(reason) => {
                for open_id in invalid_ack_references(event) {
                    if invalid_opens.contains_key(&open_id) {
                        acknowledgments
                            .ignored_by_open
                            .entry(open_id)
                            .or_default()
                            .push(ignored_event(event, reason.clone()));
                    }
                }
            }
        }
    }

    for accepted in acknowledgments.accepted_by_open.values_mut() {
        accepted.sort_by(event_order);
    }
    acknowledgments
}

fn validate_invalid_ack<'a>(
    event: &'a Event,
    invalid_opens: &BTreeMap<String, InvalidOpenAuthority>,
) -> Result<ValidatedInvalidAck<'a>, String> {
    validate_envelope(event, HANDOFF_ACK_INVALID)?;
    let context = exactly_one_tag(event, "h")?;
    let owner = event_owner(event)?;

    let tagged_ids = event
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some("e")
                && values.get(3).map(String::as_str) == Some("invalid"))
            .then(|| values.get(1).cloned())
            .flatten()
        })
        .collect::<Vec<_>>();
    if tagged_ids.is_empty()
        || tagged_ids
            .iter()
            .any(|event_id| !is_lower_hex(event_id, 64))
    {
        return Err(
            "invalid acknowledgment must carry lowercase 64-character e/invalid tags".into(),
        );
    }
    if tagged_ids.len() > MAX_INVALID_ACK_TARGETS {
        return Err(format!(
            "invalid acknowledgment cannot target more than {MAX_INVALID_ACK_TARGETS} opens"
        ));
    }
    let tagged_set = tagged_ids.iter().cloned().collect::<BTreeSet<_>>();
    if tagged_set.len() != tagged_ids.len() {
        return Err("invalid acknowledgment contains duplicate e/invalid tags".into());
    }

    let content = content_object(event)?;
    if content.get("status").and_then(Value::as_str) != Some("acknowledged-invalid") {
        return Err("invalid acknowledgment status must be acknowledged-invalid".into());
    }
    let content_ids = content
        .get("open_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| "invalid acknowledgment content must contain open_ids".to_string())?;
    if content_ids.len() > MAX_INVALID_ACK_TARGETS {
        return Err(format!(
            "invalid acknowledgment cannot target more than {MAX_INVALID_ACK_TARGETS} opens"
        ));
    }
    let content_ids = content_ids
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|event_id| is_lower_hex(event_id, 64))
                .map(str::to_owned)
                .ok_or_else(|| {
                    "invalid acknowledgment open_ids must be lowercase 64-character event ids"
                        .to_string()
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let content_set = content_ids.iter().cloned().collect::<BTreeSet<_>>();
    if content_set.len() != content_ids.len() {
        return Err("invalid acknowledgment contains duplicate open_ids".into());
    }
    if content_set != tagged_set {
        return Err("invalid acknowledgment open_ids do not match its e/invalid tags".into());
    }

    for open_id in &content_set {
        let open = invalid_opens.get(open_id).ok_or_else(|| {
            format!("invalid acknowledgment target {open_id} is not an invalid open")
        })?;
        if context != open.context {
            return Err("invalid acknowledgment targets opens from different h contexts".into());
        }
        if event.created_at.as_secs() <= open.created_at {
            return Err("invalid acknowledgment must post after every targeted open".into());
        }
        if owner != open.opener_owner_pubkey {
            return Err(
                "invalid acknowledgment is not authorized by every opener's owner identity".into(),
            );
        }
    }
    Ok(ValidatedInvalidAck {
        event,
        open_ids: content_set,
    })
}

fn validate_claim<'a>(event: &'a Event, open: &OpenAuthority) -> Result<AcceptedEvent<'a>, String> {
    validate_envelope(event, HANDOFF_CLAIM)?;
    same_context(event, open)?;
    if event.created_at.as_secs() <= open.created_at {
        return Err("claim must post after its open".into());
    }
    if linked_event(event, "root").as_deref() != Some(open.id.as_str()) {
        return Err("claim must identify its open with an e/root tag".into());
    }
    if !open.allowed_claimants.contains(&event.pubkey) {
        return Err("claim signer is not p-tagged by the open".into());
    }
    event_owner(event)?;
    Ok(AcceptedEvent { event })
}

fn validate_return<'a>(
    event: &'a Event,
    open: &OpenAuthority,
    claim: &Event,
) -> Result<AcceptedEvent<'a>, String> {
    validate_envelope(event, HANDOFF_RETURN)?;
    same_context(event, open)?;
    if event.created_at.as_secs() <= claim.created_at.as_secs() {
        return Err("return must post after its accepted claim".into());
    }
    if event.pubkey != claim.pubkey {
        return Err("return signer is not the cryptographic claimant".into());
    }
    if linked_event(event, "root").as_deref() != Some(open.id.as_str()) {
        return Err("return must identify its open with an e/root tag".into());
    }
    if linked_event(event, "claim").as_deref() != Some(claim.id.to_hex().as_str()) {
        return Err("return must identify its accepted claim with an e/claim tag".into());
    }
    let content = content_object(event)?;
    if content.get("claim_id").and_then(Value::as_str) != Some(claim.id.to_hex().as_str()) {
        return Err("return content must restate the accepted claim_id".into());
    }
    if !matches!(
        content.get("status").and_then(Value::as_str),
        Some("done" | "failed")
    ) {
        return Err("return status must be done or failed".into());
    }
    event_owner(event)?;
    Ok(AcceptedEvent { event })
}

fn validate_close<'a>(
    event: &'a Event,
    open: &OpenAuthority,
    returned: &Event,
) -> Result<AcceptedEvent<'a>, String> {
    validate_envelope(event, HANDOFF_CLOSE)?;
    same_context(event, open)?;
    if event.created_at.as_secs() <= returned.created_at.as_secs() {
        return Err("close must post after the return it verifies".into());
    }
    if linked_event(event, "root").as_deref() != Some(open.id.as_str()) {
        return Err("close must identify its open with an e/root tag".into());
    }
    if linked_event(event, "return").as_deref() != Some(returned.id.to_hex().as_str()) {
        return Err("close must identify the current return with an e/return tag".into());
    }
    let content = content_object(event)?;
    if content.get("return_id").and_then(Value::as_str) != Some(returned.id.to_hex().as_str()) {
        return Err("close content must restate the verified return_id".into());
    }
    let owner = event_owner(event)?;
    if owner != open.opener_owner_pubkey {
        return Err("close is not authorized by the opener's owner identity".into());
    }
    Ok(AcceptedEvent { event })
}

fn validate_envelope(event: &Event, expected_lifecycle: &str) -> Result<(), String> {
    if event.kind.as_u16() != KIND_TEXT_NOTE {
        return Err("handoff event must be kind 1".into());
    }
    verify_event(event).map_err(|_| "event signature or id is invalid".to_string())?;
    let lifecycle_tags = tag_values(event, "t")
        .into_iter()
        .filter(|value| value.starts_with("handoff:"))
        .collect::<Vec<_>>();
    if lifecycle_tags.as_slice() != [expected_lifecycle] {
        return Err(format!(
            "event must carry exactly one {expected_lifecycle} lifecycle tag"
        ));
    }
    exactly_one_tag(event, "h")?;
    Ok(())
}

fn event_owner(event: &Event) -> Result<PublicKey, String> {
    let auth_tags = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("auth"))
        .collect::<Vec<_>>();
    let Some(tag) = auth_tags.first() else {
        return Ok(event.pubkey);
    };
    if auth_tags.len() != 1 {
        return Err("event must carry at most one owner-attestation tag".into());
    }
    let tag_json =
        serde_json::to_string(tag.as_slice()).map_err(|_| "auth tag is not JSON".to_string())?;
    let owner = buzz_sdk::nip_oa::verify_auth_tag(&tag_json, &event.pubkey)
        .map_err(|_| "owner-attestation signature is invalid".to_string())?;
    let conditions = tag
        .as_slice()
        .get(2)
        .ok_or_else(|| "owner-attestation conditions are missing".to_string())?;
    if !conditions_cover(conditions, event.kind.as_u16(), event.created_at.as_secs()) {
        return Err("owner-attestation conditions do not cover this event".into());
    }
    Ok(owner)
}

fn verify_owner_attestation_request(
    request: &OwnerAttestationRequest,
) -> Result<PublicKey, String> {
    if !is_lower_hex(&request.signer_pubkey, 64) {
        return Err("signer_pubkey must be a lowercase 64-character public key".into());
    }
    let signer = PublicKey::from_hex(&request.signer_pubkey)
        .map_err(|_| "signer_pubkey is not a valid public key".to_string())?;
    if request.auth_tag.len() != 4 || request.auth_tag.first().map(String::as_str) != Some("auth") {
        return Err("owner-attestation must be a four-element auth tag".into());
    }
    let tag_json = serde_json::to_string(&request.auth_tag)
        .map_err(|_| "owner-attestation tag is not JSON".to_string())?;
    let owner = buzz_sdk::nip_oa::verify_auth_tag(&tag_json, &signer)
        .map_err(|_| "owner-attestation signature is invalid".to_string())?;
    if !conditions_cover(&request.auth_tag[2], request.kind, request.created_at) {
        return Err("owner-attestation conditions do not cover this event".into());
    }
    Ok(owner)
}

fn conditions_cover(conditions: &str, kind: u16, created_at: u64) -> bool {
    conditions.split('&').all(|clause| {
        if clause.is_empty() {
            true
        } else if let Some(value) = clause.strip_prefix("kind=") {
            value
                .parse::<u16>()
                .is_ok_and(|required_kind| required_kind == kind)
        } else if let Some(value) = clause.strip_prefix("created_at<") {
            value.parse::<u64>().is_ok_and(|limit| created_at < limit)
        } else if let Some(value) = clause.strip_prefix("created_at>") {
            value.parse::<u64>().is_ok_and(|limit| created_at > limit)
        } else {
            false
        }
    })
}

fn same_context(event: &Event, open: &OpenAuthority) -> Result<(), String> {
    if exactly_one_tag(event, "h")? != open.context {
        return Err("lifecycle event does not share the open's h context".into());
    }
    Ok(())
}

fn content_object(event: &Event) -> Result<serde_json::Map<String, Value>, String> {
    let content: Value = serde_json::from_str(&event.content)
        .map_err(|_| "event content is not JSON".to_string())?;
    content
        .as_object()
        .cloned()
        .ok_or_else(|| "event content must be a JSON object".to_string())
}

fn nonempty_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a str, String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{field} must be a non-empty string"))
}

fn lifecycle(event: &Event) -> Option<&str> {
    event.tags.iter().find_map(|tag| {
        let values = tag.as_slice();
        let value = values.get(1)?.as_str();
        (values.first().map(String::as_str) == Some("t") && value.starts_with("handoff:"))
            .then_some(value)
    })
}

fn tag_values(event: &Event, name: &str) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some(name))
                .then(|| values.get(1).cloned())
                .flatten()
        })
        .collect()
}

fn exactly_one_tag(event: &Event, name: &str) -> Result<String, String> {
    let values = tag_values(event, name);
    if values.len() != 1 || values[0].is_empty() {
        return Err(format!("event must carry exactly one non-empty {name} tag"));
    }
    Ok(values[0].clone())
}

fn references(event: &Event, event_id: &str) -> bool {
    event.tags.iter().any(|tag| {
        let values = tag.as_slice();
        values.first().map(String::as_str) == Some("e")
            && values.get(1).map(String::as_str) == Some(event_id)
    })
}

fn linked_event(event: &Event, marker: &str) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let values = tag.as_slice();
        (values.first().map(String::as_str) == Some("e")
            && values.get(3).map(String::as_str) == Some(marker))
        .then(|| values.get(1).cloned())
        .flatten()
    })
}

fn invalid_ack_references(event: &Event) -> BTreeSet<String> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let values = tag.as_slice();
            (values.first().map(String::as_str) == Some("e")
                && values.get(3).map(String::as_str) == Some("invalid"))
            .then(|| values.get(1).cloned())
            .flatten()
            .filter(|event_id| is_lower_hex(event_id, 64))
        })
        .collect()
}

fn event_order(left: &AcceptedEvent<'_>, right: &AcceptedEvent<'_>) -> std::cmp::Ordering {
    left.event
        .created_at
        .cmp(&right.event.created_at)
        .then_with(|| left.event.id.cmp(&right.event.id))
}

fn state_from_authority(
    authority: OpenAuthority,
    state: LifecycleState,
    claim: Option<&Event>,
    returned: Option<&Event>,
    close: Option<&Event>,
    conflicting_claims: Vec<String>,
    ignored: Vec<IgnoredEvent>,
) -> HandoffState {
    let updated_at = close
        .or(returned)
        .or(claim)
        .map_or(authority.created_at, |event| event.created_at.as_secs());
    HandoffState {
        open_id: authority.id,
        title: authority.title,
        context: authority.context,
        opener_pubkey: authority.opener_pubkey.to_hex(),
        opener_owner_pubkey: authority.opener_owner_pubkey.to_hex(),
        allowed_claimants: authority
            .allowed_claimants
            .into_iter()
            .map(|pubkey| pubkey.to_hex())
            .collect(),
        created_at: authority.created_at,
        updated_at,
        state,
        claim_id: claim.map(|event| event.id.to_hex()),
        claimant_pubkey: claim.map(|event| event.pubkey.to_hex()),
        claim_created_at: claim.map(|event| event.created_at.as_secs()),
        return_id: returned.map(|event| event.id.to_hex()),
        return_created_at: returned.map(|event| event.created_at.as_secs()),
        close_id: close.map(|event| event.id.to_hex()),
        close_created_at: close.map(|event| event.created_at.as_secs()),
        acknowledgment_id: None,
        acknowledgment_created_at: None,
        conflicting_claims,
        ignored,
    }
}

fn ignored_event(event: &Event, reason: impl Into<String>) -> IgnoredEvent {
    IgnoredEvent {
        id: event.id.to_hex(),
        reason: reason.into(),
    }
}

fn open_title(event: &Event) -> String {
    serde_json::from_str::<Value>(&event.content)
        .ok()
        .and_then(|content| {
            content
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "(invalid handoff)".into())
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use buzz_sdk::nip_oa;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use serde_json::json;

    use super::*;

    const CONTEXT: &str = "5b2ab726-c2b0-4bce-bcb9-bf5f7f883f0b";
    const BASE: &str = "6efeba90cf0b972c716c2ffd679740ac685fe75d";

    fn auth_tag(owner: &Keys, agent: &Keys) -> Tag {
        let json = nip_oa::compute_auth_tag(owner, &agent.public_key(), "kind=1")
            .expect("auth tag computes");
        nip_oa::parse_auth_tag(&json).expect("auth tag parses")
    }

    fn signed(
        keys: &Keys,
        lifecycle: &str,
        created_at: u64,
        tags: Vec<Tag>,
        content: Value,
    ) -> Event {
        EventBuilder::new(Kind::TextNote, content.to_string())
            .custom_created_at(Timestamp::from_secs(created_at))
            .tags(
                [
                    vec![
                        Tag::parse(["t", lifecycle]).expect("t tag"),
                        Tag::parse(["h", CONTEXT]).expect("h tag"),
                    ],
                    tags,
                ]
                .concat(),
            )
            .sign_with_keys(keys)
            .expect("event signs")
    }

    fn open(opener: &Keys, claimant_pubkeys: &[PublicKey]) -> Event {
        signed(
            opener,
            HANDOFF_OPEN,
            10,
            claimant_pubkeys
                .iter()
                .map(|pubkey| Tag::parse(["p", pubkey.to_hex().as_str()]).expect("p tag"))
                .collect(),
            json!({
                "title": "secure handoff",
                "scope": "only handoff files",
                "base_commit": BASE,
                "acceptance": "all cases pass",
                "embodiment": {
                    "stdin": "closed",
                    "network": "none",
                    "trust": "explicit",
                    "tooling": "cargo"
                }
            }),
        )
    }

    fn claim(claimant: &Keys, open: &Event, at: u64, runner_label: &str) -> Event {
        signed(
            claimant,
            HANDOFF_CLAIM,
            at,
            vec![Tag::parse(["e", open.id.to_hex().as_str(), "", "root"]).expect("root e tag")],
            json!({"runner": runner_label}),
        )
    }

    fn returned(claimant: &Keys, open: &Event, claim: &Event, at: u64) -> Event {
        signed(
            claimant,
            HANDOFF_RETURN,
            at,
            vec![
                Tag::parse(["e", open.id.to_hex().as_str(), "", "root"]).expect("root e tag"),
                Tag::parse(["e", claim.id.to_hex().as_str(), "", "claim"]).expect("claim e tag"),
            ],
            json!({
                "status": "done",
                "evidence": "tests pass",
                "artifacts": [],
                "claim_id": claim.id.to_hex()
            }),
        )
    }

    fn close(verifier: &Keys, owner: &Keys, open: &Event, returned: &Event, at: u64) -> Event {
        signed(
            verifier,
            HANDOFF_CLOSE,
            at,
            vec![
                Tag::parse(["e", open.id.to_hex().as_str(), "", "root"]).expect("root e tag"),
                Tag::parse(["e", returned.id.to_hex().as_str(), "", "return"])
                    .expect("return e tag"),
                auth_tag(owner, verifier),
            ],
            json!({"return_id": returned.id.to_hex(), "note": "verified"}),
        )
    }

    fn invalid_open(opener: &Keys, owner: &Keys, at: u64) -> Event {
        signed(
            opener,
            HANDOFF_OPEN,
            at,
            vec![auth_tag(owner, opener)],
            json!({
                "title": "legacy handoff",
                "scope": "historical",
                "base_commit": "6efeba90c",
                "acceptance": "already delivered",
                "embodiment": {
                    "stdin": "closed",
                    "network": "none",
                    "trust": "historical",
                    "tooling": "none"
                }
            }),
        )
    }

    fn acknowledge_invalid(verifier: &Keys, owner: &Keys, opens: &[&Event], at: u64) -> Event {
        let open_ids = opens
            .iter()
            .map(|open| open.id.to_hex())
            .collect::<Vec<_>>();
        let mut tags = opens
            .iter()
            .map(|open| {
                Tag::parse(["e", open.id.to_hex().as_str(), "", "invalid"]).expect("invalid e tag")
            })
            .collect::<Vec<_>>();
        tags.push(auth_tag(owner, verifier));
        signed(
            verifier,
            HANDOFF_ACK_INVALID,
            at,
            tags,
            json!({
                "status": "acknowledged-invalid",
                "reason": "pre-hardening archival record",
                "open_ids": open_ids
            }),
        )
    }

    #[test]
    fn accepts_authorized_causal_lifecycle() {
        let owner = Keys::generate();
        let verifier = Keys::generate();
        let claimant = Keys::generate();
        let open = open(&owner, &[claimant.public_key()]);
        let claim = claim(&claimant, &open, 20, "ignored-label");
        let returned = returned(&claimant, &open, &claim, 30);
        let close = close(&verifier, &owner, &open, &returned, 40);

        let state = reduce_handoff(
            &open,
            &[open.clone(), claim, returned.clone(), close.clone()],
        );
        assert_eq!(state.state, LifecycleState::Closed);
        assert_eq!(state.close_id, Some(close.id.to_hex()));
        assert_eq!(state.return_id, Some(returned.id.to_hex()));
    }

    #[test]
    fn ignores_spoofed_labels_and_wrong_signer_returns() {
        let owner = Keys::generate();
        let claimant = Keys::generate();
        let stranger = Keys::generate();
        let open = open(&owner, &[claimant.public_key()]);
        let spoofed_claim = claim(&stranger, &open, 20, "claude-code");
        let claim = claim(&claimant, &open, 21, "anything");
        let wrong_return = returned(&stranger, &open, &claim, 30);

        let state = reduce_handoff(
            &open,
            &[open.clone(), spoofed_claim, claim.clone(), wrong_return],
        );
        assert_eq!(state.state, LifecycleState::Claimed);
        assert_eq!(state.claimant_pubkey, Some(claimant.public_key().to_hex()));
        assert_eq!(state.ignored.len(), 2);
    }

    #[test]
    fn reports_deterministic_conflict_for_distinct_authorized_claimants() {
        let owner = Keys::generate();
        let first = Keys::generate();
        let second = Keys::generate();
        let open = open(&owner, &[first.public_key(), second.public_key()]);
        let first_claim = claim(&first, &open, 20, "same-label");
        let second_claim = claim(&second, &open, 20, "same-label");

        let state = reduce_handoff(
            &open,
            &[open.clone(), second_claim.clone(), first_claim.clone()],
        );
        assert_eq!(state.state, LifecycleState::Conflict);
        let mut expected = vec![first_claim.id.to_hex(), second_claim.id.to_hex()];
        expected.sort();
        assert_eq!(state.conflicting_claims, expected);
    }

    #[test]
    fn stale_close_cannot_override_a_later_return() {
        let owner = Keys::generate();
        let verifier = Keys::generate();
        let claimant = Keys::generate();
        let open = open(&owner, &[claimant.public_key()]);
        let claim = claim(&claimant, &open, 20, "claimant");
        let first_return = returned(&claimant, &open, &claim, 30);
        let close = close(&verifier, &owner, &open, &first_return, 40);
        let later_return = returned(&claimant, &open, &claim, 50);

        let state = reduce_handoff(
            &open,
            &[
                open.clone(),
                claim,
                first_return,
                close,
                later_return.clone(),
            ],
        );
        assert_eq!(state.state, LifecycleState::Returned);
        assert_eq!(state.return_id, Some(later_return.id.to_hex()));
        assert_eq!(state.close_id, None);
    }

    #[test]
    fn rejects_wrong_return_link_and_unauthorized_close() {
        let owner = Keys::generate();
        let claimant = Keys::generate();
        let stranger = Keys::generate();
        let open = open(&owner, &[claimant.public_key()]);
        let accepted_claim = claim(&claimant, &open, 20, "claimant");
        let other_claim = claim(&claimant, &open, 21, "duplicate");
        let wrong_link_return = returned(&claimant, &open, &other_claim, 30);
        let returned = returned(&claimant, &open, &accepted_claim, 31);
        let unauthorized_close = signed(
            &stranger,
            HANDOFF_CLOSE,
            40,
            vec![
                Tag::parse(["e", open.id.to_hex().as_str(), "", "root"]).expect("root e tag"),
                Tag::parse(["e", returned.id.to_hex().as_str(), "", "return"])
                    .expect("return e tag"),
            ],
            json!({"return_id": returned.id.to_hex()}),
        );

        let state = reduce_handoff(
            &open,
            &[
                open.clone(),
                accepted_claim,
                other_claim,
                wrong_link_return,
                returned,
                unauthorized_close,
            ],
        );
        assert_eq!(state.state, LifecycleState::Returned);
        assert!(state.ignored.len() >= 3);
    }

    #[test]
    fn rejects_an_open_with_a_forged_owner_attestation() {
        let owner = Keys::generate();
        let opener = Keys::generate();
        let claimant = Keys::generate();
        let forged_signature = "a".repeat(128);
        let event = signed(
            &opener,
            HANDOFF_OPEN,
            10,
            vec![
                Tag::parse(["p", claimant.public_key().to_hex().as_str()]).expect("p tag"),
                Tag::parse([
                    "auth",
                    owner.public_key().to_hex().as_str(),
                    "kind=1",
                    forged_signature.as_str(),
                ])
                .expect("auth-shaped tag"),
            ],
            json!({
                "title": "forged open",
                "scope": "none",
                "base_commit": BASE,
                "acceptance": "must fail",
                "embodiment": {
                    "stdin": "closed",
                    "network": "none",
                    "trust": "none",
                    "tooling": "none"
                }
            }),
        );

        let state = reduce_handoff(&event, std::slice::from_ref(&event));
        assert_eq!(state.state, LifecycleState::Invalid);
        assert!(state.ignored[0].reason.contains("signature"));
    }

    #[test]
    fn preflights_owner_attestation_signature_and_conditions() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let tag = auth_tag(&owner, &agent);
        let request = OwnerAttestationRequest {
            signer_pubkey: agent.public_key().to_hex(),
            auth_tag: tag.as_slice().to_vec(),
            kind: KIND_TEXT_NOTE,
            created_at: 20,
        };
        assert_eq!(
            verify_owner_attestation_request(&request).expect("attestation verifies"),
            owner.public_key()
        );

        let expiring_json =
            nip_oa::compute_auth_tag(&owner, &agent.public_key(), "kind=1&created_at<20")
                .expect("auth tag computes");
        let expired = OwnerAttestationRequest {
            signer_pubkey: agent.public_key().to_hex(),
            auth_tag: nip_oa::parse_auth_tag(&expiring_json)
                .expect("auth tag parses")
                .as_slice()
                .to_vec(),
            kind: KIND_TEXT_NOTE,
            created_at: 20,
        };
        assert!(verify_owner_attestation_request(&expired)
            .expect_err("expired attestation is rejected")
            .contains("conditions"));
    }

    #[test]
    fn requires_strictly_causal_transition_times() {
        let owner = Keys::generate();
        let claimant = Keys::generate();
        let open = open(&owner, &[claimant.public_key()]);
        let premature_claim = claim(&claimant, &open, 10, "claimant");
        let state = reduce_handoff(&open, &[open.clone(), premature_claim]);
        assert_eq!(state.state, LifecycleState::Open);

        let claim = claim(&claimant, &open, 20, "claimant");
        let premature_return = returned(&claimant, &open, &claim, 20);
        let state = reduce_handoff(&open, &[open.clone(), claim.clone(), premature_return]);
        assert_eq!(state.state, LifecycleState::Claimed);

        let returned = returned(&claimant, &open, &claim, 30);
        let verifier = Keys::generate();
        let premature_close = close(&verifier, &owner, &open, &returned, 30);
        let state = reduce_handoff(&open, &[open.clone(), claim, returned, premature_close]);
        assert_eq!(state.state, LifecycleState::Returned);
    }

    #[test]
    fn requires_explicit_root_markers_and_claim_id_restatement() {
        let owner = Keys::generate();
        let claimant = Keys::generate();
        let open = open(&owner, &[claimant.public_key()]);
        let unmarked_claim = signed(
            &claimant,
            HANDOFF_CLAIM,
            20,
            vec![Tag::parse(["e", open.id.to_hex().as_str()]).expect("unmarked e tag")],
            json!({"runner": "legacy"}),
        );
        let state = reduce_handoff(&open, &[open.clone(), unmarked_claim]);
        assert_eq!(state.state, LifecycleState::Open);

        let claim = claim(&claimant, &open, 21, "claimant");
        let missing_claim_id = signed(
            &claimant,
            HANDOFF_RETURN,
            30,
            vec![
                Tag::parse(["e", open.id.to_hex().as_str(), "", "root"]).expect("root e tag"),
                Tag::parse(["e", claim.id.to_hex().as_str(), "", "claim"]).expect("claim e tag"),
            ],
            json!({"status": "done", "evidence": "missing content binding", "artifacts": []}),
        );
        let state = reduce_handoff(&open, &[open.clone(), claim, missing_claim_id]);
        assert_eq!(state.state, LifecycleState::Claimed);
    }

    #[test]
    fn duplicate_claims_from_one_signer_choose_the_earliest() {
        let owner = Keys::generate();
        let claimant = Keys::generate();
        let open = open(&owner, &[claimant.public_key()]);
        let first = claim(&claimant, &open, 20, "first-label");
        let duplicate = claim(&claimant, &open, 21, "second-label");

        let state = reduce_handoff(&open, &[open.clone(), duplicate, first.clone()]);
        assert_eq!(state.state, LifecycleState::Claimed);
        assert_eq!(state.claim_id, Some(first.id.to_hex()));
        assert_eq!(state.ignored.len(), 1);
    }

    #[test]
    fn owner_can_acknowledge_exact_invalid_open_ids_without_validating_them() {
        let owner = Keys::generate();
        let opener = Keys::generate();
        let verifier = Keys::generate();
        let first = invalid_open(&opener, &owner, 10);
        let second = invalid_open(&opener, &owner, 11);
        let acknowledgment = acknowledge_invalid(&verifier, &owner, &[&first, &second], 20);
        let events = [first.clone(), second.clone(), acknowledgment.clone()];

        for open in [&first, &second] {
            let state = reduce_handoff(open, &events);
            assert_eq!(state.state, LifecycleState::AcknowledgedInvalid);
            assert_eq!(state.acknowledgment_id, Some(acknowledgment.id.to_hex()));
            assert_eq!(state.opener_owner_pubkey, owner.public_key().to_hex());
            assert!(state.ignored[0]
                .reason
                .contains("base_commit must be a canonical"));
        }
    }

    #[test]
    fn rejects_an_entire_acknowledgment_if_any_target_is_not_eligible() {
        let owner = Keys::generate();
        let opener = Keys::generate();
        let verifier = Keys::generate();
        let claimant = Keys::generate();
        let invalid = invalid_open(&opener, &owner, 10);
        let valid = open(&owner, &[claimant.public_key()]);
        let acknowledgment = acknowledge_invalid(&verifier, &owner, &[&invalid, &valid], 20);

        let state = reduce_handoff(&invalid, &[invalid.clone(), valid, acknowledgment.clone()]);
        assert_eq!(state.state, LifecycleState::Invalid);
        assert_eq!(state.acknowledgment_id, None);
        assert!(state.ignored.iter().any(|ignored| {
            ignored.id == acknowledgment.id.to_hex()
                && ignored.reason.contains("is not an invalid open")
        }));

        let missing_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let missing_target = signed(
            &verifier,
            HANDOFF_ACK_INVALID,
            21,
            vec![
                Tag::parse(["e", invalid.id.to_hex().as_str(), "", "invalid"])
                    .expect("invalid e tag"),
                Tag::parse(["e", missing_id, "", "invalid"]).expect("missing e tag"),
                auth_tag(&owner, &verifier),
            ],
            json!({
                "status": "acknowledged-invalid",
                "reason": "must reject atomically",
                "open_ids": [invalid.id.to_hex(), missing_id]
            }),
        );
        let state = reduce_handoff(&invalid, &[invalid.clone(), missing_target.clone()]);
        assert_eq!(state.state, LifecycleState::Invalid);
        assert!(state.ignored.iter().any(|ignored| {
            ignored.id == missing_target.id.to_hex()
                && ignored.reason.contains("is not an invalid open")
        }));

        let other_owner = Keys::generate();
        let other_opener = Keys::generate();
        let other_invalid = invalid_open(&other_opener, &other_owner, 11);
        let mixed_owner = acknowledge_invalid(&verifier, &owner, &[&invalid, &other_invalid], 22);
        let state = reduce_handoff(
            &invalid,
            &[invalid.clone(), other_invalid, mixed_owner.clone()],
        );
        assert_eq!(state.state, LifecycleState::Invalid);
        assert!(state.ignored.iter().any(|ignored| {
            ignored.id == mixed_owner.id.to_hex()
                && ignored.reason.contains("every opener's owner identity")
        }));

        let later_invalid = invalid_open(&opener, &owner, 30);
        let premature = acknowledge_invalid(&verifier, &owner, &[&invalid, &later_invalid], 29);
        let state = reduce_handoff(
            &invalid,
            &[invalid.clone(), later_invalid, premature.clone()],
        );
        assert_eq!(state.state, LifecycleState::Invalid);
        assert!(state.ignored.iter().any(|ignored| {
            ignored.id == premature.id.to_hex()
                && ignored.reason.contains("after every targeted open")
        }));
    }

    #[test]
    fn unauthorized_or_mismatched_invalid_acknowledgments_are_ignored() {
        let owner = Keys::generate();
        let opener = Keys::generate();
        let stranger = Keys::generate();
        let open = invalid_open(&opener, &owner, 10);
        let unauthorized = signed(
            &stranger,
            HANDOFF_ACK_INVALID,
            20,
            vec![
                Tag::parse(["e", open.id.to_hex().as_str(), "", "invalid"]).expect("invalid e tag")
            ],
            json!({
                "status": "acknowledged-invalid",
                "reason": "unauthorized",
                "open_ids": [open.id.to_hex()]
            }),
        );
        let mismatched = signed(
            &opener,
            HANDOFF_ACK_INVALID,
            21,
            vec![
                Tag::parse(["e", open.id.to_hex().as_str(), "", "invalid"]).expect("invalid e tag"),
                auth_tag(&owner, &opener),
            ],
            json!({
                "status": "acknowledged-invalid",
                "reason": "mismatched",
                "open_ids": ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]
            }),
        );

        let state = reduce_handoff(&open, &[open.clone(), unauthorized, mismatched]);
        assert_eq!(state.state, LifecycleState::Invalid);
        assert_eq!(state.acknowledgment_id, None);
        assert_eq!(state.ignored.len(), 3);
    }
}
