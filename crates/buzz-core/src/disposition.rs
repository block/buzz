//! NIP-AD: Agent Disposition — the shared verifier and lifecycle derivation.
//!
//! This module is the single authority for two questions every consumer asks:
//!
//! 1. **Is this a valid v1 request, and if so whose obligation is it?**
//!    ([`classify_request`])
//! 2. **Given a request and some candidate dispositions, what is its state?**
//!    ([`derive_obligation`])
//!
//! It exists because the previous implementation answered both questions
//! independently in `buzz-cli` and in the desktop timeline, and the two
//! answers drifted — different candidate sets, different binding rules,
//! different conflict semantics. A protocol whose whole purpose is
//! verifiable accountability cannot have two first-party consumers that
//! disagree about what was verified. The TypeScript mirror lives at
//! `desktop/src/shared/lib/disposition.ts`, and both are pinned to the same
//! JSON corpus (`docs/nips/nip-ad-conformance.json`) so they cannot drift
//! again silently.
//!
//! Zero I/O: everything here is a pure function over already-fetched events,
//! so it can run in the relay, the CLI, a test, or an external auditor.
//!
//! See `docs/nips/NIP-AD.md`.

use serde::{Deserialize, Serialize};

/// The one obligation a marked request creates in v1.
///
/// v1 deliberately requires **exactly one** agent target, which is what makes
/// this key unambiguous. A multi-target request has no single well-defined
/// outcome — two agents legitimately answering would either look like a
/// conflict or let one agent's answer discharge another's obligation — so v1
/// classifies those as unsupported rather than guessing. When per-agent
/// splitting arrives, this becomes one obligation per target and the rest of
/// the module is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Obligation {
    /// Event id of the request message.
    pub request_id: String,
    /// Channel the request lives in (its `h` tag).
    pub channel_id: String,
    /// Who asked (the request's author).
    pub requester_pubkey: String,
    /// The single agent expected to answer.
    pub target_agent_pubkey: String,
}

/// Why a `["t","request"]`-marked event is malformed.
///
/// These are NOT agent failures and MUST NOT be reported as unanswered
/// requests. An event nobody can validly answer is a client bug, and counting
/// it as a gap would blame an agent for it — a permanent false gap that never
/// clears.
///
/// Distinct from [`UnsupportedRequest`]: these events are *wrong*, whereas an
/// unsupported one expresses a coherent intent v1 cannot yet represent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvalidRequest {
    /// Marked as a request but names no agent — nothing was asked of anyone.
    MissingAgentTarget,
    /// An `agent` tag that isn't canonical 64-char lowercase hex.
    MalformedAgentTarget,
    /// No `h` tag, so the request isn't scoped to a channel.
    MissingChannel,
    /// More than one `h` tag. One channel would govern authorization while
    /// another still rode along for tag matching — the same ambiguity kind
    /// 44300 rejects on the disposition side, rejected here too.
    MultipleChannels,
    /// The named agent isn't also `p`-mentioned, so it was never addressed
    /// in the way that actually routes a message to it.
    TargetNotMentioned,
    /// The same `agent` target repeated. Canonical events keep independent
    /// implementations and audit output simple, so a duplicate is rejected
    /// rather than quietly folded into one target.
    DuplicateAgentTarget,
    /// The event's kind is not one v1 accepts requests on. Without this the
    /// CLI and the desktop can fetch different candidate universes and
    /// legitimately disagree about the same channel.
    UnsupportedKind,
}

/// A well-formed request whose intent v1 cannot represent.
///
/// Kept separate from [`InvalidRequest`] because the remedy differs: an
/// invalid request is a bug to fix, an unsupported one is a feature to build.
/// Both block a clean channel claim; only one implies anybody did anything
/// wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedRequest {
    /// Names two or more distinct agents. The event cannot say whether either
    /// target may discharge one shared obligation, both owe independent
    /// answers, or one owns it and the other is consulted — so v1 refuses to
    /// guess. See [`Obligation`].
    MultipleAgentTargets,
}

/// Result of classifying any event. Total: every event lands in exactly one
/// arm, so no caller needs a marker pre-check of its own.
///
/// Totality is the point. An earlier version required callers to check the
/// marker first and classify second, and the ACP harness grew its own
/// looser predicate instead — it accepted uppercase targets, multi-target
/// requests, and targets that were never `p`-mentioned, so it would do real
/// work and publish dispositions for events every consumer called invalid.
/// One total function with no precondition removes the temptation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestClass {
    /// Not marked `["t","request"]` — ordinary traffic, no obligation.
    NotRequest,
    /// Marked but malformed — see [`InvalidRequest`].
    Invalid(InvalidRequest),
    /// Marked and well-formed, but not representable in v1.
    Unsupported(UnsupportedRequest),
    /// A usable v1 obligation.
    Valid(Box<Obligation>),
}

impl RequestClass {
    /// The obligation, when this is a valid request.
    pub fn obligation(&self) -> Option<&Obligation> {
        match self {
            Self::Valid(o) => Some(o),
            _ => None,
        }
    }
}

/// A disposition state. Terminal states settle an obligation; non-terminal
/// ones leave it open.
///
/// `Responded` is the honest counterpart to a bare runtime end-of-turn: the
/// agent produced a response, which is all the harness can observe. It is
/// deliberately NOT `Completed` — a clean turn covers asking a clarifying
/// question, reporting it couldn't finish, or answering one message of a
/// batch, none of which accomplished the request. `Completed` requires an
/// explicit per-request assertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispositionState {
    /// Explicitly asserted done. Terminal.
    Completed,
    /// Explicitly declined, with a reason. Terminal.
    Refused,
    /// The agent responded; no completion was asserted. Non-terminal.
    Responded,
    /// Technical failure — the turn did not end cleanly. Non-terminal.
    Errored,
}

impl DispositionState {
    /// The wire value carried in the `disposition` tag and `content`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Refused => "refused",
            Self::Responded => "responded",
            Self::Errored => "errored",
        }
    }

    /// Parse a wire value. Unknown values are rejected rather than coerced —
    /// a disposition whose state we don't understand must not be silently
    /// treated as some other state.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "completed" => Some(Self::Completed),
            "refused" => Some(Self::Refused),
            "responded" => Some(Self::Responded),
            "errored" => Some(Self::Errored),
            _ => None,
        }
    }

    /// Whether this state settles the obligation.
    ///
    /// `Errored` and `Responded` are non-terminal: both mean "something
    /// happened, the request may still reach a settled outcome." This is
    /// what makes `errored → completed` and `responded → completed` legal
    /// repairs rather than contradictions.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Refused)
    }

    /// Every valid wire value, for validators and tests.
    pub const ALL: [Self; 4] = [
        Self::Completed,
        Self::Refused,
        Self::Responded,
        Self::Errored,
    ];
}

/// Something worth surfacing about a history that does not change its
/// outcome.
///
/// Warnings are strictly diagnostic. An earlier version made every one of
/// these force the obligation out of its settled state and render as
/// "disputed", which defeated the point of categorizing them: a duplicate
/// delivery and a genuine contradiction produced identical, equally
/// destructive results. Only [`EffectiveOutcome::Disputed`] — two *opposing*
/// terminal claims — is a real dispute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryWarning {
    /// The same terminal state recorded more than once by distinct events.
    /// Redundant, not contradictory — usually a retry that re-published.
    DuplicateTerminal,
    /// A **non-terminal** disposition sorts after a terminal one under
    /// `(created_at, id)` — a stale or late weak observation.
    ///
    /// Deliberately **not** named for a causal claim. The ordering is
    /// deterministic but publisher-supplied, so this cannot establish that
    /// anything was actually *written after* settlement — only that it sorts
    /// later. Naming it `PostTerminalWrite` (as an earlier version did) told
    /// users something the algorithm cannot know; a real causal claim needs
    /// a trusted receive sequence or an attempt ordinal.
    ///
    /// Scoped to non-terminal followers so the warnings never overlap: a
    /// terminal after a terminal is already fully described by
    /// [`Self::DuplicateTerminal`] (same state) or by
    /// [`EffectiveOutcome::Disputed`] (opposing states). Two warnings for one
    /// event is noise, and noise is what made the previous single `conflict`
    /// boolean useless.
    OrderedAfterTerminal,
}

/// What actually became of an obligation.
///
/// Separate from the raw latest observation on purpose. Deriving state as
/// "latest event wins" made `terminal` a lie: a late `errored` silently
/// reopened a settled obligation. Here terminal claims are **absorbing** —
/// once something settles, a later weaker observation is recorded as a
/// warning and cannot reopen it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "state")]
pub enum EffectiveOutcome {
    /// Nothing bound at all — a real gap.
    Unanswered,
    /// Answered, not settled. Carries `Responded` or `Errored`.
    Open(DispositionState),
    /// Settled. Carries `Completed` or `Refused`.
    Settled(DispositionState),
    /// Both terminal states claimed. Genuinely contradictory; no settled
    /// answer exists.
    Disputed,
}

impl EffectiveOutcome {
    /// The obligation is done. Only [`Self::Settled`] qualifies — a dispute
    /// is not a resolution, and neither is an open state.
    pub fn is_settled(self) -> bool {
        matches!(self, Self::Settled(_))
    }

    /// Something bound but the obligation is not settled.
    pub fn is_open(self) -> bool {
        matches!(self, Self::Open(_))
    }
}

/// An obligation's derived history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedObligation {
    /// The obligation this was derived for.
    pub obligation: Obligation,
    /// What became of it, under terminal-absorbing semantics. This is what
    /// every accounting and display surface must use.
    pub outcome: EffectiveOutcome,
    /// The last bound state by `(created_at, id)`, regardless of absorption.
    /// Retained for audit and debugging — it answers "what arrived last",
    /// which is a different question from "what is true". Never use it to
    /// decide whether an obligation is done.
    pub latest_observation: Option<DispositionState>,
    /// Reason from the disposition that determined [`Self::outcome`] — the
    /// settling event when settled, otherwise the latest bound one. A
    /// refusal's stated reason must survive a later stray write.
    pub reason: String,
    /// Diagnostics that do not change the outcome, deduplicated and ordered.
    pub warnings: Vec<HistoryWarning>,
}

impl DerivedObligation {
    /// Settled. The only condition under which a surface may claim done.
    pub fn is_resolved(&self) -> bool {
        self.outcome.is_settled()
    }

    /// Answered but not settled.
    pub fn is_open(&self) -> bool {
        self.outcome.is_open()
    }

    /// Nothing bound at all: a real gap.
    pub fn is_unanswered(&self) -> bool {
        matches!(self.outcome, EffectiveOutcome::Unanswered)
    }

    /// Contradictory terminal claims.
    pub fn is_disputed(&self) -> bool {
        matches!(self.outcome, EffectiveOutcome::Disputed)
    }
}

/// Minimal view of a Nostr event this module needs. Callers adapt from
/// `nostr::Event`, `serde_json::Value`, or a test fixture — keeping this
/// module free of any particular event representation is what lets the relay,
/// the CLI, and an external auditor share one verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventView<'a> {
    /// Event id (64 lowercase hex).
    pub id: &'a str,
    /// Signing pubkey (64 lowercase hex).
    pub pubkey: &'a str,
    /// Event kind. Required so request classification can reject kinds v1
    /// does not accept requests on — without it, two consumers querying
    /// different kind sets produce different accounting for one channel and
    /// both look correct.
    pub kind: u16,
    /// Whole-second Nostr timestamp.
    pub created_at: i64,
    /// Raw event content.
    pub content: &'a str,
    /// Tag rows, each `[key, value, ...]`.
    pub tags: &'a [Vec<String>],
}

impl<'a> EventView<'a> {
    /// Values borrow from the event's own data (`'a`), not from `&self`, so
    /// callers can hold them past the borrow of the view.
    fn tag_values<'s>(&'s self, key: &'s str) -> impl Iterator<Item = &'a str> + 's {
        self.tags
            .iter()
            .filter(move |t| t.len() >= 2 && t[0] == key)
            .map(|t| t[1].as_str())
    }

    fn first_tag(&self, key: &str) -> Option<&'a str> {
        self.tag_values(key).next()
    }

    fn has_tag_pair(&self, key: &str, value: &str) -> bool {
        self.tag_values(key).any(|v| v == value)
    }
}

/// Canonical 64-character lowercase hex.
///
/// Case is part of the contract, not a formatting preference: the harness and
/// the readers compare these values, and a case-insensitive writer paired
/// with a case-sensitive reader silently produces dispositions that no one
/// binds. One canonical form, compared exactly, everywhere.
pub fn is_canonical_hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Event kinds v1 accepts `["t","request"]` markers on.
///
/// Centralized so every query builder and every classifier derives its
/// candidate universe from one list. The CLI previously queried only
/// `KIND_STREAM_MESSAGE` while the desktop timeline carried several message
/// and job kinds, so even identical derivation logic could produce different
/// accounting for the same channel.
pub const REQUEST_KINDS: [u16; 1] = [crate::kind::KIND_STREAM_MESSAGE as u16];

/// Whether v1 accepts a request marker on this kind.
pub fn is_request_kind(kind: u16) -> bool {
    REQUEST_KINDS.contains(&kind)
}

/// Whether an event carries the NIP-AD request marker.
///
/// A marker check alone says nothing about validity — prefer
/// [`classify_request`], which is total.
pub fn is_marked_request(event: &EventView<'_>) -> bool {
    event.has_tag_pair("t", "request")
}

/// Classify any event. Total — no precondition, no caller-side marker check.
///
/// The order of checks is deliberate: kind and channel first (an event on the
/// wrong kind or with an ambiguous channel is unusable regardless of who it
/// names), then targeting.
pub fn classify_request(event: &EventView<'_>) -> RequestClass {
    if !is_marked_request(event) {
        return RequestClass::NotRequest;
    }
    if !is_request_kind(event.kind) {
        return RequestClass::Invalid(InvalidRequest::UnsupportedKind);
    }

    let channels: Vec<&str> = event.tag_values("h").collect();
    let channel_id = match channels.as_slice() {
        [] => return RequestClass::Invalid(InvalidRequest::MissingChannel),
        [single] => *single,
        _ => return RequestClass::Invalid(InvalidRequest::MultipleChannels),
    };

    let agents: Vec<&str> = event.tag_values("agent").collect();
    if agents.is_empty() {
        return RequestClass::Invalid(InvalidRequest::MissingAgentTarget);
    }

    // **Malformed beats unsupported.** Every target is validated before
    // cardinality is judged, so a request naming one real agent and one
    // garbage value is a malformed event — not "a feature v1 cannot
    // represent". Returning Unsupported first (as an earlier version did)
    // filed client bugs under "not anyone's fault" and told the sender their
    // perfectly reasonable request needed a future protocol version.
    //
    // `p`-mention is checked in the same pass: `p` is what actually routes a
    // message to a principal, so an `agent` tag without it names a target
    // that will never be asked — an obligation nobody could ever discharge.
    for target in &agents {
        if !is_canonical_hex64(target) {
            return RequestClass::Invalid(InvalidRequest::MalformedAgentTarget);
        }
        if !event.has_tag_pair("p", target) {
            return RequestClass::Invalid(InvalidRequest::TargetNotMentioned);
        }
    }

    let mut unique: Vec<&str> = agents.clone();
    unique.sort_unstable();
    unique.dedup();
    let target = match (agents.len(), unique.len()) {
        (1, _) => agents[0],
        // Distinct, well-formed, reachable targets: a coherent intent v1
        // cannot represent.
        (_, u) if u > 1 => {
            return RequestClass::Unsupported(UnsupportedRequest::MultipleAgentTargets)
        }
        // One target, written twice. Rejected rather than folded: this
        // document promises exactly one `agent` tag, and canonical events
        // keep independent implementations and audit output simple.
        _ => return RequestClass::Invalid(InvalidRequest::DuplicateAgentTarget),
    };

    RequestClass::Valid(Box::new(Obligation {
        request_id: event.id.to_string(),
        channel_id: channel_id.to_string(),
        requester_pubkey: event.pubkey.to_string(),
        target_agent_pubkey: target.to_string(),
    }))
}

/// Why an event is not a structurally valid kind:44300 disposition.
///
/// Every variant corresponds to a rule in NIP-AD's "Event" and "Content"
/// sections: if the NIP states a structural requirement, it is enforced in
/// [`validate_disposition_event`] and named here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidDisposition {
    /// Not kind 44300 at all.
    WrongKind,
    /// Not exactly one `e` tag.
    RequestIdCardinality,
    /// Not exactly one `h` tag.
    ChannelCardinality,
    /// Not exactly one `p` tag.
    RequesterCardinality,
    /// Not exactly one `disposition` tag.
    StateCardinality,
    /// The `e` value is not canonical 64-char lowercase hex.
    MalformedRequestId,
    /// The `p` value is not canonical 64-char lowercase hex.
    MalformedRequester,
    /// The `disposition` tag value is not a recognized state.
    UnknownState,
    /// `content` is not parseable JSON, or is not an object.
    ContentNotObject,
    /// `content.disposition` is missing or disagrees with the tag.
    ContentStateMismatch,
    /// `content.reason` is absent.
    MissingReason,
    /// `content.reason` is present but not a string.
    ReasonNotString,
    /// `content.request_id` is present but not a string.
    RequestIdNotString,
    /// `content.request_id` disagrees with the `e` tag.
    ContentRequestIdMismatch,
}

/// A structurally valid kind:44300 event, with its fields extracted once.
///
/// Holding this value is proof that the full NIP-AD envelope and content
/// contract was checked. Nothing may contribute to an obligation's state
/// without one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalDisposition<'a> {
    /// The disposition event's own id.
    pub event_id: &'a str,
    /// Who signed it.
    pub signer: &'a str,
    /// Whole-second Nostr timestamp.
    pub created_at: i64,
    /// The request it answers (`e`).
    pub request_id: &'a str,
    /// The channel it is scoped to (`h`).
    pub channel_id: &'a str,
    /// The requesting principal it names (`p`).
    pub requester_pubkey: &'a str,
    /// The state, agreed between tag and content.
    pub state: DispositionState,
    /// `content.reason` — required to be present, permitted to be empty.
    pub reason: String,
}

/// Validate an event against the **complete** NIP-AD structural contract.
///
/// This is the single validity boundary. The relay calls it at ingest, and
/// every consumer calls it (via [`bind_disposition`]) before an event may
/// affect any obligation. That sharing is the point, not an optimization.
///
/// It exists because the two sides had drifted apart in the worst possible
/// direction. The relay enforced kind, exact `e`/`h`/`p`/`disposition`
/// cardinality, and a required string `reason`; the consumer-side binder
/// checked none of them — it read the *first* matching tag and ignored the
/// rest. An event carrying two `e` tags, or no `reason`, or **not even of
/// kind 44300**, was rejected by the relay and simultaneously accepted as a
/// settling disposition by the verifier this module advertises for auditing
/// imported history where no relay stood in the way. Signature verification
/// does not help: a valid signature can cover a perfectly well-signed
/// non-disposition.
pub fn validate_disposition_event<'a>(
    event: &EventView<'a>,
) -> Result<CanonicalDisposition<'a>, InvalidDisposition> {
    if event.kind != crate::kind::KIND_AGENT_DISPOSITION as u16 {
        return Err(InvalidDisposition::WrongKind);
    }

    // Exactly one of each. `first_tag` semantics are deliberately not used
    // here: silently taking the first of several is precisely how the two
    // validators came to disagree.
    let exactly_one = |key: &str, err: InvalidDisposition| -> Result<&'a str, InvalidDisposition> {
        let mut values = event.tag_values(key);
        let first = values.next().ok_or(err)?;
        if values.next().is_some() {
            return Err(err);
        }
        Ok(first)
    };

    let request_id = exactly_one("e", InvalidDisposition::RequestIdCardinality)?;
    let channel_id = exactly_one("h", InvalidDisposition::ChannelCardinality)?;
    let requester_pubkey = exactly_one("p", InvalidDisposition::RequesterCardinality)?;
    let state_value = exactly_one("disposition", InvalidDisposition::StateCardinality)?;

    if !is_canonical_hex64(request_id) {
        return Err(InvalidDisposition::MalformedRequestId);
    }
    if !is_canonical_hex64(requester_pubkey) {
        return Err(InvalidDisposition::MalformedRequester);
    }
    let state = DispositionState::parse(state_value).ok_or(InvalidDisposition::UnknownState)?;

    let body: serde_json::Value =
        serde_json::from_str(event.content).map_err(|_| InvalidDisposition::ContentNotObject)?;
    let body = body
        .as_object()
        .ok_or(InvalidDisposition::ContentNotObject)?;

    if body.get("disposition").and_then(|v| v.as_str()) != Some(state.as_str()) {
        return Err(InvalidDisposition::ContentStateMismatch);
    }
    let reason = match body.get("reason") {
        Some(v) => v
            .as_str()
            .ok_or(InvalidDisposition::ReasonNotString)?
            .to_string(),
        None => return Err(InvalidDisposition::MissingReason),
    };
    if let Some(rid) = body.get("request_id") {
        let rid = rid.as_str().ok_or(InvalidDisposition::RequestIdNotString)?;
        if rid != request_id {
            return Err(InvalidDisposition::ContentRequestIdMismatch);
        }
    }

    Ok(CanonicalDisposition {
        event_id: event.id,
        signer: event.pubkey,
        created_at: event.created_at,
        request_id,
        channel_id,
        requester_pubkey,
        state,
        reason,
    })
}

/// Why a candidate disposition does not bind to an obligation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindFailure {
    /// The event is not a structurally valid disposition at all.
    Invalid(InvalidDisposition),
    /// `e` tag missing or not this obligation's request.
    RequestMismatch,
    /// `h` tag missing or a different channel than the request's.
    ChannelMismatch,
    /// `p` tag missing or not the request's actual author.
    RequesterMismatch,
    /// Signed by someone other than the obligation's target agent. This is
    /// the cross-principal spoof check.
    NotTargetAgent,
}

/// Whether `disposition` validly answers `obligation`.
///
/// Two stages, in this order:
///
/// 1. **Structural validity** — [`validate_disposition_event`]. An event that
///    is not a canonical kind:44300 disposition can never contribute to an
///    obligation, whatever it claims. This stage used to be missing entirely
///    here while the relay enforced it, so the two sides disagreed about what
///    a disposition even *is*.
/// 2. **Binding** — does this specific valid disposition answer this specific
///    obligation? Request, channel, requester, and target-agent signer all
///    have to match. The target-agent clause is the cross-principal check:
///    without it any channel member, including a merely `p`-mentioned human,
///    can close an agent's obligation.
///
/// The relay performs stage 1 and deliberately not stage 2 — binding would
/// require fetching a second event mid-validation. So every consumer must do
/// stage 2, identically, which is why this function exists rather than three
/// near-copies.
pub fn bind_disposition(
    obligation: &Obligation,
    disposition: &EventView<'_>,
) -> Result<DispositionState, BindFailure> {
    let canonical = validate_disposition_event(disposition).map_err(BindFailure::Invalid)?;
    bind_canonical(obligation, &canonical)
}

/// Stage 2 alone, for callers that already validated.
pub fn bind_canonical(
    obligation: &Obligation,
    disposition: &CanonicalDisposition<'_>,
) -> Result<DispositionState, BindFailure> {
    if disposition.request_id != obligation.request_id {
        return Err(BindFailure::RequestMismatch);
    }
    if disposition.channel_id != obligation.channel_id {
        return Err(BindFailure::ChannelMismatch);
    }
    if disposition.requester_pubkey != obligation.requester_pubkey {
        return Err(BindFailure::RequesterMismatch);
    }
    if disposition.signer != obligation.target_agent_pubkey {
        return Err(BindFailure::NotTargetAgent);
    }
    Ok(disposition.state)
}

fn reason_of(disposition: &EventView<'_>) -> String {
    serde_json::from_str::<serde_json::Value>(disposition.content)
        .ok()
        .and_then(|v| {
            v.get("reason")
                .and_then(|r| r.as_str())
                .map(std::string::ToString::to_string)
        })
        .unwrap_or_default()
}

/// Derive an obligation's state from candidate dispositions.
///
/// Candidates are filtered by [`bind_disposition`], deduplicated by event id
/// (a relay result merged from several queries can legitimately contain the
/// same event twice — without this, one duplicate delivery reads as a
/// disputed outcome), then ordered by `(created_at, id)`.
///
/// **That ordering is deterministic, not causal.** `created_at` is
/// whole-second and publisher-supplied, so a later publication can carry an
/// earlier timestamp. The anomalies below therefore detect *timestamp-order*
/// contradictions, which is a strictly weaker claim than detecting a real
/// post-terminal write. Under v1's single authoritative writer per
/// obligation, the two coincide in practice; a system with genuinely
/// concurrent writers would need an explicit attempt ordinal instead.
pub fn derive_obligation(
    obligation: &Obligation,
    candidates: &[EventView<'_>],
) -> DerivedObligation {
    let mut bound: Vec<(&EventView<'_>, DispositionState)> = Vec::new();
    let mut seen_ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for candidate in candidates {
        let Ok(state) = bind_disposition(obligation, candidate) else {
            continue;
        };
        if !seen_ids.insert(candidate.id) {
            continue;
        }
        bound.push((candidate, state));
    }

    bound.sort_by(|a, b| (a.0.created_at, a.0.id).cmp(&(b.0.created_at, b.0.id)));

    let latest_observation = bound.last().map(|(_, s)| *s);
    let has_completed = bound.iter().any(|(_, s)| *s == DispositionState::Completed);
    let has_refused = bound.iter().any(|(_, s)| *s == DispositionState::Refused);

    // Terminal-absorbing. The first terminal claim decides the outcome; a
    // later non-terminal observation is a warning, never a reopening.
    let first_terminal = bound.iter().position(|(_, s)| s.is_terminal());
    let outcome = match (has_completed, has_refused, first_terminal) {
        (true, true, _) => EffectiveOutcome::Disputed,
        (_, _, Some(idx)) => EffectiveOutcome::Settled(bound[idx].1),
        (_, _, None) => match latest_observation {
            Some(state) => EffectiveOutcome::Open(state),
            None => EffectiveOutcome::Unanswered,
        },
    };

    let mut warnings = Vec::new();
    // "Duplicate" means the SAME terminal state twice. Counting any two
    // terminals would fire on `completed` + `refused`, which is a
    // contradiction (already `Disputed`), not a duplicate.
    let duplicated = [DispositionState::Completed, DispositionState::Refused]
        .iter()
        .any(|state| bound.iter().filter(|(_, s)| s == state).count() > 1);
    if duplicated {
        warnings.push(HistoryWarning::DuplicateTerminal);
    }
    if let Some(idx) = first_terminal {
        if bound[idx + 1..].iter().any(|(_, s)| !s.is_terminal()) {
            warnings.push(HistoryWarning::OrderedAfterTerminal);
        }
    }

    // The settling event's reason, not the latest event's — a stray write
    // after a refusal must not blank out why the agent refused.
    let reason = match first_terminal {
        Some(idx) if !matches!(outcome, EffectiveOutcome::Disputed) => reason_of(bound[idx].0),
        _ => bound.last().map(|(e, _)| reason_of(e)).unwrap_or_default(),
    };

    DerivedObligation {
        obligation: obligation.clone(),
        outcome,
        latest_observation,
        reason,
        warnings,
    }
}

/// How much of the record a caller actually examined.
///
/// Both sides are required, which an earlier single `scope_complete` flag
/// got wrong: a caller could paginate every request, fetch one page of
/// dispositions, see a `completed`, miss the later `refused` on page two,
/// and truthfully set the old flag while reporting a disputed obligation as
/// resolved. Completeness of the request set alone proves nothing about the
/// completeness of any obligation's history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Coverage {
    /// Every marked request in scope was fetched.
    pub requests_complete: bool,
    /// Every disposition for those requests was fetched.
    pub dispositions_complete: bool,
}

impl Coverage {
    /// Both sides complete. Only then may a channel-wide claim be made.
    pub fn is_complete(self) -> bool {
        self.requests_complete && self.dispositions_complete
    }

    /// Coverage a partial reader (one page, a loaded UI window) must use.
    pub fn partial() -> Self {
        Self::default()
    }

    /// Coverage a reader that paginated both sides to exhaustion may use.
    pub fn complete() -> Self {
        Self {
            requests_complete: true,
            dispositions_complete: true,
        }
    }
}

/// A stored disposition that named an obligation but did not bind to it.
///
/// Excluded from state — that is the whole point of binding — but reported
/// separately so an auditor can see spoof attempts rather than having them
/// silently vanish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedClaim {
    /// The unbound disposition's event id.
    pub event_id: String,
    /// The request it referenced via `e`.
    pub referenced_request: String,
    /// Who signed it.
    pub signer: String,
    /// Why it did not bind.
    pub failure: BindFailure,
}

/// Accounting over a set of requests, with its coverage stated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accounting {
    /// Settled obligations.
    pub settled: Vec<String>,
    /// Answered, not settled (`responded` or `errored`).
    pub open: Vec<String>,
    /// Nothing bound at all — a real gap.
    pub unanswered: Vec<String>,
    /// Contradictory terminal claims.
    pub disputed: Vec<String>,
    /// Malformed marked events, with why. Client faults, never agent
    /// failures.
    pub invalid_requests: Vec<(String, InvalidRequest)>,
    /// Well-formed marked events v1 cannot represent, with why. Not anyone's
    /// fault; still blocks a clean claim.
    pub unsupported_requests: Vec<(String, UnsupportedRequest)>,
    /// Unbound dispositions, for diagnostics only.
    pub rejected_claims: Vec<RejectedClaim>,
    /// What the caller actually examined.
    pub coverage: Coverage,
}

impl Accounting {
    /// Every request examined is settled, over provably complete coverage.
    /// The only condition under which a surface may claim the channel is
    /// clean.
    ///
    /// `invalid_requests` and `unsupported_requests` block this too, which is
    /// easy to miss: neither is an agent failure, but both are marked events
    /// nobody can ever answer. Reporting such a channel as clean hides the
    /// problem behind a green check.
    pub fn all_resolved(&self) -> bool {
        self.coverage.is_complete()
            && self.open.is_empty()
            && self.unanswered.is_empty()
            && self.disputed.is_empty()
            && self.invalid_requests.is_empty()
            && self.unsupported_requests.is_empty()
    }

    /// Requests examined, across every bucket.
    pub fn total(&self) -> usize {
        self.settled.len()
            + self.open.len()
            + self.unanswered.len()
            + self.disputed.len()
            + self.invalid_requests.len()
            + self.unsupported_requests.len()
    }
}

/// Build accounting from candidate requests and dispositions.
///
/// `requests` may contain any events — classification is total, and
/// non-requests are ignored. Dispositions are grouped by `e` tag once, so
/// this is O(requests + dispositions) rather than re-scanning every
/// disposition for every request: any channel member can store structurally
/// valid but unbound dispositions, so the quadratic version was an avoidable
/// denial-of-service surface.
pub fn account(
    requests: &[EventView<'_>],
    dispositions: &[EventView<'_>],
    coverage: Coverage,
) -> Accounting {
    let mut acc = Accounting {
        settled: Vec::new(),
        open: Vec::new(),
        unanswered: Vec::new(),
        disputed: Vec::new(),
        invalid_requests: Vec::new(),
        unsupported_requests: Vec::new(),
        rejected_claims: Vec::new(),
        coverage,
    };

    let mut by_request: std::collections::HashMap<&str, Vec<EventView<'_>>> =
        std::collections::HashMap::new();
    for d in dispositions {
        if let Some(e) = d.first_tag("e") {
            by_request.entry(e).or_default().push(d.clone());
        }
    }
    let empty: Vec<EventView<'_>> = Vec::new();

    for request in requests {
        match classify_request(request) {
            RequestClass::NotRequest => {}
            RequestClass::Invalid(reason) => {
                acc.invalid_requests.push((request.id.to_string(), reason));
            }
            RequestClass::Unsupported(reason) => {
                acc.unsupported_requests
                    .push((request.id.to_string(), reason));
            }
            RequestClass::Valid(obligation) => {
                let candidates = by_request.get(request.id).unwrap_or(&empty);
                let derived = derive_obligation(&obligation, candidates);
                let id = obligation.request_id.clone();
                match derived.outcome {
                    EffectiveOutcome::Settled(_) => acc.settled.push(id),
                    EffectiveOutcome::Open(_) => acc.open.push(id),
                    EffectiveOutcome::Unanswered => acc.unanswered.push(id),
                    EffectiveOutcome::Disputed => acc.disputed.push(id),
                }
                for candidate in candidates {
                    if let Err(failure) = bind_disposition(&obligation, candidate) {
                        acc.rejected_claims.push(RejectedClaim {
                            event_id: candidate.id.to_string(),
                            referenced_request: obligation.request_id.clone(),
                            signer: candidate.pubkey.to_string(),
                            failure,
                        });
                    }
                }
            }
        }
    }

    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OTHER_AGENT: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const HUMAN: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const REQUESTER: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    const CHANNEL: &str = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";
    const REQ_ID: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    fn tags(pairs: &[(&str, &str)]) -> Vec<Vec<String>> {
        pairs
            .iter()
            .map(|(k, v)| vec![(*k).to_string(), (*v).to_string()])
            .collect()
    }

    fn request_tags() -> Vec<Vec<String>> {
        tags(&[
            ("h", CHANNEL),
            ("t", "request"),
            ("agent", AGENT),
            ("p", AGENT),
            ("p", HUMAN),
        ])
    }

    fn request<'a>(tags: &'a [Vec<String>]) -> EventView<'a> {
        EventView {
            id: REQ_ID,
            pubkey: REQUESTER,
            kind: REQUEST_KINDS[0],
            created_at: 100,
            content: "@agent do the thing",
            tags,
        }
    }

    fn disposition_tags(state: &str) -> Vec<Vec<String>> {
        tags(&[
            ("e", REQ_ID),
            ("h", CHANNEL),
            ("p", REQUESTER),
            ("disposition", state),
        ])
    }

    fn disposition<'a>(
        id: &'a str,
        signer: &'a str,
        created_at: i64,
        tags: &'a [Vec<String>],
        content: &'a str,
    ) -> EventView<'a> {
        EventView {
            id,
            pubkey: signer,
            kind: crate::kind::KIND_AGENT_DISPOSITION as u16,
            created_at,
            content,
            tags,
        }
    }

    fn valid_obligation() -> Obligation {
        Obligation {
            request_id: REQ_ID.to_string(),
            channel_id: CHANNEL.to_string(),
            requester_pubkey: REQUESTER.to_string(),
            target_agent_pubkey: AGENT.to_string(),
        }
    }

    #[test]
    fn a_canonical_request_classifies_to_one_obligation() {
        let t = request_tags();
        assert_eq!(
            classify_request(&request(&t)),
            RequestClass::Valid(Box::new(valid_obligation()))
        );
    }

    #[test]
    fn a_request_naming_no_agent_is_invalid_not_an_obligation() {
        // Must never be counted as an unanswered agent failure: nothing was
        // asked of anyone, so no agent can ever clear it.
        let t = tags(&[("h", CHANNEL), ("t", "request"), ("p", HUMAN)]);
        assert_eq!(
            classify_request(&request(&t)),
            RequestClass::Invalid(InvalidRequest::MissingAgentTarget)
        );
    }

    #[test]
    fn a_multi_target_request_is_unsupported_in_v1() {
        // Two agents legitimately answering would either read as a conflict
        // or let one discharge the other's obligation. v1 refuses to guess.
        let t = tags(&[
            ("h", CHANNEL),
            ("t", "request"),
            ("agent", AGENT),
            ("agent", OTHER_AGENT),
            ("p", AGENT),
            ("p", OTHER_AGENT),
        ]);
        assert_eq!(
            classify_request(&request(&t)),
            RequestClass::Unsupported(UnsupportedRequest::MultipleAgentTargets)
        );
    }

    #[test]
    fn a_repeated_identical_target_is_rejected_as_non_canonical() {
        // Previously folded into one target. The prose promises exactly one
        // `agent` tag; accepting a duplicate made the prose false and left
        // independent implementations to guess.
        let t = tags(&[
            ("h", CHANNEL),
            ("t", "request"),
            ("agent", AGENT),
            ("agent", AGENT),
            ("p", AGENT),
        ]);
        assert_eq!(
            classify_request(&request(&t)),
            RequestClass::Invalid(InvalidRequest::DuplicateAgentTarget)
        );
    }

    #[test]
    fn an_unmarked_event_is_not_a_request_at_all() {
        // Totality: no caller needs its own marker pre-check, which is how
        // the harness ended up with a looser predicate of its own.
        let t = tags(&[("h", CHANNEL), ("agent", AGENT), ("p", AGENT)]);
        assert_eq!(classify_request(&request(&t)), RequestClass::NotRequest);
    }

    #[test]
    fn a_marker_on_an_unsupported_kind_is_invalid() {
        let t = request_tags();
        let mut view = request(&t);
        view.kind = crate::kind::KIND_AGENT_DISPOSITION as u16;
        assert_eq!(
            classify_request(&view),
            RequestClass::Invalid(InvalidRequest::UnsupportedKind)
        );
    }

    #[test]
    fn two_channel_tags_are_invalid_not_first_wins() {
        // The same ambiguity kind 44300 rejects on the disposition side: one
        // channel would govern authorization while the other still rode
        // along for tag matching.
        let t = tags(&[
            ("h", CHANNEL),
            ("h", "99999999-0e2d-4cfe-bd6e-567eb169db9f"),
            ("t", "request"),
            ("agent", AGENT),
            ("p", AGENT),
        ]);
        assert_eq!(
            classify_request(&request(&t)),
            RequestClass::Invalid(InvalidRequest::MultipleChannels)
        );
    }

    #[test]
    fn an_uppercase_target_is_malformed_not_silently_accepted() {
        // The case that previously made the harness emit events no reader
        // would bind: harness compared case-insensitively, readers exactly.
        let upper = AGENT.to_ascii_uppercase();
        let t = tags(&[
            ("h", CHANNEL),
            ("t", "request"),
            ("agent", &upper),
            ("p", &upper),
        ]);
        assert_eq!(
            classify_request(&request(&t)),
            RequestClass::Invalid(InvalidRequest::MalformedAgentTarget)
        );
    }

    #[test]
    fn a_target_that_is_not_p_mentioned_is_invalid() {
        // `p` is what routes the message. An `agent` tag without it names a
        // target that never receives the request.
        let t = tags(&[
            ("h", CHANNEL),
            ("t", "request"),
            ("agent", AGENT),
            ("p", HUMAN),
        ]);
        assert_eq!(
            classify_request(&request(&t)),
            RequestClass::Invalid(InvalidRequest::TargetNotMentioned)
        );
    }

    #[test]
    fn the_target_agent_binds_and_a_mentioned_human_does_not() {
        let ob = valid_obligation();
        let dt = disposition_tags("completed");
        let body = r#"{"disposition":"completed","reason":""}"#;

        assert_eq!(
            bind_disposition(&ob, &disposition("d1", AGENT, 200, &dt, body)),
            Ok(DispositionState::Completed)
        );
        // The headline spoof: a `p`-mentioned human signing a completion.
        assert_eq!(
            bind_disposition(&ob, &disposition("d2", HUMAN, 200, &dt, body)),
            Err(BindFailure::NotTargetAgent)
        );
        assert_eq!(
            bind_disposition(&ob, &disposition("d3", OTHER_AGENT, 200, &dt, body)),
            Err(BindFailure::NotTargetAgent)
        );
    }

    #[test]
    fn a_disposition_from_another_channel_or_requester_does_not_bind() {
        let ob = valid_obligation();
        let body = r#"{"disposition":"completed","reason":""}"#;

        let wrong_channel = tags(&[
            ("e", REQ_ID),
            ("h", "99999999-0000-0000-0000-000000000000"),
            ("p", REQUESTER),
            ("disposition", "completed"),
        ]);
        assert_eq!(
            bind_disposition(&ob, &disposition("d1", AGENT, 200, &wrong_channel, body)),
            Err(BindFailure::ChannelMismatch)
        );

        let wrong_requester = tags(&[
            ("e", REQ_ID),
            ("h", CHANNEL),
            ("p", HUMAN),
            ("disposition", "completed"),
        ]);
        assert_eq!(
            bind_disposition(&ob, &disposition("d2", AGENT, 200, &wrong_requester, body)),
            Err(BindFailure::RequesterMismatch)
        );
    }

    #[test]
    fn content_disagreeing_with_its_tags_does_not_bind() {
        let ob = valid_obligation();
        let dt = disposition_tags("completed");
        assert_eq!(
            bind_disposition(
                &ob,
                // Recognizable state, but the event contradicts itself —
                // distinct from an unparseable one.
                &disposition(
                    "d1",
                    AGENT,
                    200,
                    &dt,
                    r#"{"disposition":"refused","reason":""}"#
                )
            ),
            Err(BindFailure::Invalid(
                InvalidDisposition::ContentStateMismatch
            ))
        );
        assert_eq!(
            bind_disposition(
                &ob,
                &disposition(
                    "d2",
                    AGENT,
                    200,
                    &dt,
                    r#"{"disposition":"completed","reason":"","request_id":"ffff"}"#
                )
            ),
            Err(BindFailure::Invalid(
                InvalidDisposition::ContentRequestIdMismatch
            ))
        );
    }

    #[test]
    fn a_duplicate_event_id_is_not_a_disputed_outcome() {
        // The same event delivered twice (merged query results) must not
        // read as two finalizations.
        let ob = valid_obligation();
        let dt = disposition_tags("completed");
        let body = r#"{"disposition":"completed","reason":"done"}"#;
        let d = disposition("dup", AGENT, 200, &dt, body);
        let derived = derive_obligation(&ob, &[d.clone(), d]);
        assert_eq!(
            derived.outcome,
            EffectiveOutcome::Settled(DispositionState::Completed)
        );
        assert!(derived.warnings.is_empty(), "{:?}", derived.warnings);
        assert!(derived.is_resolved());
    }

    #[test]
    fn a_repeated_terminal_warns_but_stays_settled() {
        // The finding that motivated splitting warnings from outcomes: this
        // is described as harmless and redundant, yet the previous model
        // made it unresolved and rendered it as "disputed" — identical
        // treatment to a genuine contradiction.
        let ob = valid_obligation();
        let dt = disposition_tags("completed");
        let body = r#"{"disposition":"completed","reason":""}"#;
        let derived = derive_obligation(
            &ob,
            &[
                disposition("d1", AGENT, 200, &dt, body),
                disposition("d2", AGENT, 300, &dt, body),
            ],
        );
        assert_eq!(derived.warnings, vec![HistoryWarning::DuplicateTerminal]);
        assert!(
            derived.is_resolved(),
            "a duplicate delivery is not a dispute"
        );
    }

    #[test]
    fn completed_plus_refused_is_disputed_and_never_settled() {
        let ob = valid_obligation();
        let ct = disposition_tags("completed");
        let rt = disposition_tags("refused");
        let derived = derive_obligation(
            &ob,
            &[
                disposition(
                    "d1",
                    AGENT,
                    200,
                    &ct,
                    r#"{"disposition":"completed","reason":""}"#,
                ),
                disposition(
                    "d2",
                    AGENT,
                    300,
                    &rt,
                    r#"{"disposition":"refused","reason":""}"#,
                ),
            ],
        );
        assert_eq!(derived.outcome, EffectiveOutcome::Disputed);
        assert!(!derived.is_resolved());
        assert!(derived.is_disputed());
    }

    #[test]
    fn a_late_non_terminal_cannot_reopen_a_settled_obligation() {
        // Terminal-absorbing. Previously the later `errored` won outright,
        // so a state the spec called terminal silently reopened.
        let ob = valid_obligation();
        let ct = disposition_tags("completed");
        let et = disposition_tags("errored");
        let derived = derive_obligation(
            &ob,
            &[
                disposition(
                    "d1",
                    AGENT,
                    200,
                    &ct,
                    r#"{"disposition":"completed","reason":""}"#,
                ),
                disposition(
                    "d2",
                    AGENT,
                    300,
                    &et,
                    r#"{"disposition":"errored","reason":""}"#,
                ),
            ],
        );
        assert_eq!(
            derived.outcome,
            EffectiveOutcome::Settled(DispositionState::Completed),
            "terminal claims absorb later weaker observations"
        );
        assert_eq!(
            derived.latest_observation,
            Some(DispositionState::Errored),
            "the raw latest observation is still available for audit"
        );
        assert_eq!(derived.warnings, vec![HistoryWarning::OrderedAfterTerminal]);
        assert!(derived.is_resolved());
    }

    #[test]
    fn a_refusal_reason_survives_a_later_stray_write() {
        let ob = valid_obligation();
        let rt = disposition_tags("refused");
        let et = disposition_tags("errored");
        let derived = derive_obligation(
            &ob,
            &[
                disposition(
                    "d1",
                    AGENT,
                    200,
                    &rt,
                    r#"{"disposition":"refused","reason":"outside my delegation"}"#,
                ),
                disposition(
                    "d2",
                    AGENT,
                    300,
                    &et,
                    r#"{"disposition":"errored","reason":"transport blip"}"#,
                ),
            ],
        );
        assert_eq!(derived.reason, "outside my delegation");
    }

    #[test]
    fn non_terminal_progress_then_completion_is_a_clean_repair() {
        // errored -> responded -> completed is the normal repair path and
        // must never be flagged. Guards against an over-broad rule.
        let ob = valid_obligation();
        let et = disposition_tags("errored");
        let st = disposition_tags("responded");
        let ct = disposition_tags("completed");
        let derived = derive_obligation(
            &ob,
            &[
                disposition(
                    "d1",
                    AGENT,
                    100,
                    &et,
                    r#"{"disposition":"errored","reason":""}"#,
                ),
                disposition(
                    "d2",
                    AGENT,
                    200,
                    &st,
                    r#"{"disposition":"responded","reason":""}"#,
                ),
                disposition(
                    "d3",
                    AGENT,
                    300,
                    &ct,
                    r#"{"disposition":"completed","reason":""}"#,
                ),
            ],
        );
        assert!(derived.warnings.is_empty(), "{:?}", derived.warnings);
        assert!(derived.is_resolved());
    }

    #[test]
    fn responded_alone_is_open_not_resolved() {
        // The whole point of the state: the agent answered, but nothing
        // asserted the request was accomplished.
        let ob = valid_obligation();
        let st = disposition_tags("responded");
        let derived = derive_obligation(
            &ob,
            &[disposition(
                "d1",
                AGENT,
                200,
                &st,
                r#"{"disposition":"responded","reason":""}"#,
            )],
        );
        assert_eq!(
            derived.outcome,
            EffectiveOutcome::Open(DispositionState::Responded)
        );
        assert!(!derived.is_resolved());
        assert!(derived.is_open());
    }

    #[test]
    fn same_second_ties_break_by_id_deterministically() {
        let ob = valid_obligation();
        let st = disposition_tags("responded");
        let ct = disposition_tags("completed");
        let a = disposition(
            "aaa",
            AGENT,
            200,
            &ct,
            r#"{"disposition":"completed","reason":""}"#,
        );
        let z = disposition(
            "zzz",
            AGENT,
            200,
            &st,
            r#"{"disposition":"responded","reason":""}"#,
        );
        let forward = derive_obligation(&ob, &[a.clone(), z.clone()]);
        let reversed = derive_obligation(&ob, &[z, a]);
        assert_eq!(forward.outcome, reversed.outcome);
        assert_eq!(forward.latest_observation, reversed.latest_observation);
        // `completed` sorts first by id, so it settles; the tie order is what
        // must be stable, not which state happens to win.
        assert_eq!(
            forward.outcome,
            EffectiveOutcome::Settled(DispositionState::Completed)
        );
    }

    #[test]
    fn content_that_is_not_a_json_object_never_binds() {
        // Confirmed divergence: Rust rejected `null`/`42`/`"hi"` while
        // TypeScript bound them, because both skipped body validation for a
        // non-object body — in opposite directions.
        let ob = valid_obligation();
        let ct = disposition_tags("completed");
        for body in ["null", "42", "\"hi\"", "{not json", "[]"] {
            assert_eq!(
                bind_disposition(&ob, &disposition("d1", AGENT, 200, &ct, body)),
                Err(BindFailure::Invalid(InvalidDisposition::ContentNotObject)),
                "content {body} must not bind"
            );
        }
    }

    #[test]
    fn accounting_separates_invalid_requests_from_real_gaps() {
        let good = request_tags();
        let targetless = tags(&[("h", CHANNEL), ("t", "request"), ("p", HUMAN)]);
        let good_req = request(&good);
        let bad_req = EventView {
            id: "2222222222222222222222222222222222222222222222222222222222222222",
            ..request(&targetless)
        };
        let acc = account(&[good_req, bad_req], &[], Coverage::complete());
        assert_eq!(acc.unanswered, vec![REQ_ID.to_string()]);
        assert_eq!(
            acc.invalid_requests,
            vec![(
                "2222222222222222222222222222222222222222222222222222222222222222".to_string(),
                InvalidRequest::MissingAgentTarget
            )]
        );
        assert!(!acc.all_resolved());
    }

    #[test]
    fn unsupported_requests_are_their_own_bucket_and_block_a_clean_claim() {
        let t = tags(&[
            ("h", CHANNEL),
            ("t", "request"),
            ("agent", AGENT),
            ("agent", OTHER_AGENT),
            ("p", AGENT),
            ("p", OTHER_AGENT),
        ]);
        let acc = account(&[request(&t)], &[], Coverage::complete());
        assert!(acc.invalid_requests.is_empty(), "not a malformed event");
        assert_eq!(
            acc.unsupported_requests,
            vec![(REQ_ID.to_string(), UnsupportedRequest::MultipleAgentTargets)]
        );
        assert!(acc.unanswered.is_empty(), "never an agent gap");
        assert!(!acc.all_resolved());
    }

    #[test]
    fn an_unbound_claim_is_excluded_from_state_but_reported() {
        // Spoof attempts must not vanish silently — an auditor needs to see
        // them, they just must not affect the outcome.
        let t = request_tags();
        let ct = disposition_tags("completed");
        let acc = account(
            &[request(&t)],
            &[disposition(
                "d1",
                HUMAN,
                200,
                &ct,
                r#"{"disposition":"completed","reason":""}"#,
            )],
            Coverage::complete(),
        );
        assert_eq!(acc.unanswered, vec![REQ_ID.to_string()]);
        assert_eq!(acc.rejected_claims.len(), 1);
        assert_eq!(acc.rejected_claims[0].failure, BindFailure::NotTargetAgent);
        assert_eq!(acc.rejected_claims[0].signer, HUMAN);
    }

    #[test]
    fn all_resolved_requires_both_sides_of_coverage() {
        let t = request_tags();
        let ct = disposition_tags("completed");
        let settled = [disposition(
            "d1",
            AGENT,
            200,
            &ct,
            r#"{"disposition":"completed","reason":""}"#,
        )];
        // Exhausting only the request side is not enough: an unfetched later
        // `refused` would make this obligation disputed, not settled.
        let one_sided = Coverage {
            requests_complete: true,
            dispositions_complete: false,
        };
        let acc = account(&[request(&t)], &settled, one_sided);
        assert_eq!(acc.settled.len(), 1);
        assert!(
            !acc.all_resolved(),
            "complete requests with partial dispositions cannot claim a clean channel"
        );

        let acc = account(&[request(&t)], &settled, Coverage::complete());
        assert!(acc.all_resolved());
    }

    #[test]
    fn all_resolved_requires_complete_scope() {
        let t = request_tags();
        let ct = disposition_tags("completed");
        let acc = account(
            &[request(&t)],
            &[disposition(
                "d1",
                AGENT,
                200,
                &ct,
                r#"{"disposition":"completed","reason":""}"#,
            )],
            Coverage::partial(),
        );
        assert_eq!(acc.settled.len(), 1);
        assert!(
            !acc.all_resolved(),
            "partial coverage must never claim channel-wide resolution"
        );
    }

    #[test]
    fn disposition_state_roundtrips_and_rejects_unknown() {
        for state in DispositionState::ALL {
            assert_eq!(DispositionState::parse(state.as_str()), Some(state));
        }
        assert_eq!(DispositionState::parse("maybe"), None);
        assert!(DispositionState::Completed.is_terminal());
        assert!(DispositionState::Refused.is_terminal());
        assert!(!DispositionState::Responded.is_terminal());
        assert!(!DispositionState::Errored.is_terminal());
    }
}
