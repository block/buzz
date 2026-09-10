//! MIKE-49 checkpoint 1 (Desktop fixture-only audit command): a SMALL
//! in-process fixture built ONLY from disposable `nostr::Keys::generate()`
//! keys and the reviewed `nip-am-verify` crate's cursor/verify API — no
//! real key, no real event, no relay/network read anywhere in this module.
//!
//! Reuses `tools/nip-am-verify/src/bin/pipeline_demo.rs`'s construction
//! patterns (disposable owner/agent/someone-else keys, `signed_event_at`
//! building a real signed+NIP-44-encrypted `kind:44200` event, distinct
//! `created_at` values for determinism) rather than inventing new ones —
//! this fixture is a deliberately SMALLER subset of that same shape, not a
//! different design.
//!
//! Three fixture events, in descending `created_at` (pagination visits
//! newest first):
//!
//! 1. `wrong_recipient_verify_error` (created_at 3000) — validly signed and
//!    shaped, but addressed to a DIFFERENT owner than the one this command
//!    verifies against. `verify_and_decrypt` rejects it with
//!    `VerifyError::WrongRecipient` before ever attempting decryption.
//! 2. `accepted_record` (created_at 2000) — a normal, well-formed,
//!    correctly-addressed event. `verify_and_decrypt` succeeds and
//!    `SessionCoverageTracker::observe` reports `ObserveOutcome::Accepted`
//!    (first observation for its session).
//! 3. `never_fetched_due_to_page_cap` (created_at 1000) — exists only in
//!    the fake page source, never verified. With `page_limit=2,
//!    max_pages=Some(1)`, a single `ForwardCollector::collect_new` call
//!    fetches exactly one page of 2 (events 1 and 2 above, newest first)
//!    and then stops on `StopReason::PageCapReached` — event 3 is
//!    genuinely left unfetched, proving the pagination result is
//!    incomplete, not just labeled that way.
//!
//! A SEPARATE, genuinely empty `InMemoryPageSource` (zero events) is
//! probed independently — `StopReason::Exhausted` with 0 events, the
//! honestly-different case from "capped with more data behind it."

use std::collections::HashMap;

use nip_am_verify::cursor::{FakeEvent, ForwardCollector, InMemoryPageSource, PageSource};
use nip_am_verify::{AgentTurnMetricPayload, TokenCounts, KIND_AGENT_TURN_METRIC};
use nostr::event::{Event, EventBuilder, Kind, Tag};
use nostr::key::{Keys, PublicKey};
use nostr::nips::nip44;
use nostr::types::Timestamp;

/// A fixture-only, disposable-key event plus the scenario label it proves.
pub struct FixtureEvent {
    pub scenario: &'static str,
    pub event: Event,
}

/// Everything a single `mike49_run_fixture_audit` run needs: the disposable
/// "owner" identity the command verifies against, the fake page source
/// mirroring the real relay pagination contract, and a lookup from event id
/// back to its scenario label + signed event (pagination only returns ids).
pub struct Fixture {
    pub owner: Keys,
    pub page_limit: usize,
    pub max_pages: usize,
    pub source: InMemoryPageSource,
    pub by_id: HashMap<String, FixtureEvent>,
}

/// `None` only on a defect in this fixture's own construction (a malformed
/// literal tag value) — never on live/attacker input, since there is none.
/// Propagated via `?` rather than `.expect()`/`.unwrap()`, per this repo's
/// "no new `unwrap()`/`expect()` in production paths" rule — this module
/// compiles into the production binary even though its DATA is
/// fixture-only.
fn standard_tags(agent: &Keys, owner_pubkey: PublicKey) -> Option<Vec<Tag>> {
    let p = Tag::parse(["p", &owner_pubkey.to_hex()]).ok()?;
    let a = Tag::parse(["agent", &agent.public_key().to_hex()]).ok()?;
    Some(vec![p, a])
}

/// `None` only on a defect in this fixture's own construction — see
/// [`standard_tags`]'s doc comment for why this returns `Option` rather
/// than panicking.
fn signed_event_at(
    agent: &Keys,
    recipient_pubkey: PublicKey,
    payload: &AgentTurnMetricPayload,
    created_at: u64,
) -> Option<Event> {
    let plaintext = serde_json::to_string(payload).ok()?;
    let ciphertext = nip44::encrypt(
        agent.secret_key(),
        &recipient_pubkey,
        &plaintext,
        nip44::Version::V2,
    )
    .ok()?;
    let event = EventBuilder::new(Kind::Custom(KIND_AGENT_TURN_METRIC), ciphertext)
        .tags(standard_tags(agent, recipient_pubkey)?)
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(agent)
        .ok()?;
    Some(event)
}

fn base_payload(session_id: &str, turn_seq: u64) -> AgentTurnMetricPayload {
    AgentTurnMetricPayload {
        harness: "codex-acp".into(),
        model: Some("gpt-5".into()),
        channel_id: None,
        session_id: Some(session_id.into()),
        turn_id: Some(format!("{session_id}-turn-{turn_seq}")),
        turn_seq: Some(turn_seq),
        timestamp: "2026-09-09T00:00:00.000Z".into(),
        turn: Some(TokenCounts {
            input_tokens: Some(100),
            output_tokens: Some(10),
            total_tokens: Some(110),
            cost_usd: Some(0.01),
            cache_read_tokens: None,
            cache_write_tokens: None,
        }),
        cumulative: Some(TokenCounts {
            input_tokens: Some(100 * turn_seq),
            output_tokens: Some(10 * turn_seq),
            total_tokens: Some(110 * turn_seq),
            cost_usd: Some(0.01 * turn_seq as f64),
            cache_read_tokens: None,
            cache_write_tokens: None,
        }),
        delta_reliable: true,
        stop_reason: Some("end_turn".into()),
        pricing_identity: None,
    }
}

/// Builds the small, deterministic, disposable-key fixture described in
/// this module's own doc comment. `None` only on a defect in this
/// fixture's own construction (see [`standard_tags`]'s doc comment) — a
/// real caller should treat `None` as an internal error, never retry with
/// different input (there is none to vary).
pub fn build_fixture() -> Option<Fixture> {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let someone_else = Keys::generate();

    let mut by_id = HashMap::new();
    let mut page_events = Vec::new();

    let mut add = |scenario: &'static str, event: Event| {
        page_events.push(FakeEvent {
            created_at: event.created_at.as_secs() as i64,
            id: event.id.to_hex(),
        });
        by_id.insert(event.id.to_hex(), FixtureEvent { scenario, event });
    };

    // 1. Wrong-recipient verify error: addressed to `someone_else`, not the
    // `owner` this command will verify against.
    let wrong_recipient_payload = base_payload("mike49-fixture-session", 1);
    let wrong_recipient_event = signed_event_at(
        &agent,
        someone_else.public_key(),
        &wrong_recipient_payload,
        3000,
    )?;
    add("wrong_recipient_verify_error", wrong_recipient_event);

    // 2. Accepted record: correctly addressed to `owner`, well-formed,
    // first observation for its session -> ObserveOutcome::Accepted.
    let accepted_payload = base_payload("mike49-fixture-session-accepted", 1);
    let accepted_event = signed_event_at(&agent, owner.public_key(), &accepted_payload, 2000)?;
    add("accepted_record", accepted_event);

    // 3. Never fetched: exists in the source, but the page cap below stops
    // the walk before this event's page would ever be requested.
    let never_fetched_payload = base_payload("mike49-fixture-session-never-fetched", 1);
    let never_fetched_event =
        signed_event_at(&agent, owner.public_key(), &never_fetched_payload, 1000)?;
    add("never_fetched_due_to_page_cap", never_fetched_event);

    Some(Fixture {
        owner,
        page_limit: 2,
        max_pages: 1,
        source: InMemoryPageSource::new(page_events),
        by_id,
    })
}

/// A genuinely empty fake page source — zero events, never populated.
/// Probed independently of [`build_fixture`]'s main, capped traversal to
/// prove the "empty" case is a real, distinct, exercised code path (not
/// merely asserted in prose): a single `ForwardCollector::collect_new`
/// call against this returns `StopReason::Exhausted` with 0 events on its
/// very first page fetch.
pub fn build_empty_source() -> InMemoryPageSource {
    InMemoryPageSource::new(Vec::new())
}

/// Runs a single, UNRESUMED `collect_new` call against `source` — used both
/// for the fixture's intentionally-capped main traversal and for the empty
/// probe. Deliberately does NOT loop/resume (unlike
/// `pipeline_demo.rs`'s `run_pagination`, which drains to completion) —
/// this command's whole point is to demonstrate a single run's result
/// exactly as a first-ever, uncached query would see it, cap included.
pub fn run_single_capped_call<S: PageSource>(
    source: &S,
    page_limit: usize,
    max_pages: usize,
) -> Result<nip_am_verify::cursor::CollectResult, nip_am_verify::cursor::CursorError> {
    let collector = ForwardCollector::with_caps(source, page_limit, Some(max_pages), None);
    collector.collect_new(None, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nip_am_verify::cursor::StopReason;
    use nip_am_verify::verify_and_decrypt;

    #[test]
    fn fixture_has_exactly_three_events_covering_the_required_scenarios() {
        let fixture = build_fixture().expect("fixture construction never fails in tests");
        assert_eq!(fixture.by_id.len(), 3);
        let scenarios: std::collections::BTreeSet<&str> =
            fixture.by_id.values().map(|e| e.scenario).collect();
        assert!(scenarios.contains("wrong_recipient_verify_error"));
        assert!(scenarios.contains("accepted_record"));
        assert!(scenarios.contains("never_fetched_due_to_page_cap"));
    }

    #[test]
    fn main_traversal_is_capped_and_leaves_the_third_event_unfetched() {
        let fixture = build_fixture().expect("fixture construction never fails in tests");
        let result = run_single_capped_call(&fixture.source, fixture.page_limit, fixture.max_pages)
            .expect("in-memory source never errors");
        assert_eq!(result.stop_reason, StopReason::PageCapReached);
        assert_eq!(
            result.events.len(),
            2,
            "only the first page's 2 events are fetched"
        );
        let fetched_scenarios: Vec<&str> = result
            .events
            .iter()
            .map(|e| fixture.by_id[&e.id].scenario)
            .collect();
        assert_eq!(
            fetched_scenarios,
            vec!["wrong_recipient_verify_error", "accepted_record"],
            "newest-first pagination order"
        );
        assert!(
            !result
                .events
                .iter()
                .any(|e| fixture.by_id[&e.id].scenario == "never_fetched_due_to_page_cap"),
            "the third event must genuinely never be fetched"
        );
    }

    #[test]
    fn wrong_recipient_event_fails_verification_against_the_owner() {
        let fixture = build_fixture().expect("fixture construction never fails in tests");
        let wrong_recipient = fixture
            .by_id
            .values()
            .find(|e| e.scenario == "wrong_recipient_verify_error")
            .expect("present in fixture");
        let result = verify_and_decrypt(&wrong_recipient.event, &fixture.owner);
        assert!(result.is_err());
    }

    #[test]
    fn accepted_event_verifies_successfully_against_the_owner() {
        let fixture = build_fixture().expect("fixture construction never fails in tests");
        let accepted = fixture
            .by_id
            .values()
            .find(|e| e.scenario == "accepted_record")
            .expect("present in fixture");
        let result = verify_and_decrypt(&accepted.event, &fixture.owner);
        assert!(result.is_ok());
    }

    #[test]
    fn empty_source_probe_is_exhausted_immediately_with_zero_events() {
        let empty = build_empty_source();
        let result = run_single_capped_call(&empty, 5, 1).expect("empty source never errors");
        assert_eq!(result.stop_reason, StopReason::Exhausted);
        assert!(result.events.is_empty());
    }
}
