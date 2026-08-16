/**
 * NIP-AD: Agent Disposition — the shared request/disposition verifier and
 * lifecycle derivation, mirroring `crates/buzz-core/src/disposition.rs`.
 *
 * The two implementations are pinned to one JSON corpus
 * (`docs/nips/nip-ad-conformance.json`), run by `disposition.test.mjs` here
 * and `nip_ad_conformance.rs` there. They previously drifted — different
 * candidate sets, different binding rules, different conflict semantics — and
 * the drift was invisible because each side tested itself with its own
 * fixtures. Change behavior here and the Rust corpus test fails, and vice
 * versa. That is deliberate.
 *
 * **This pins the two implementations to each other, not to the spec.** That
 * gap is real and was exploited once already: the NIP's transition table and
 * `isResolved` disagreed about whether an anomalous terminal history was
 * resolved, and every test passed because both languages agreed with each
 * other. The normative table in NIP-AD.md is now generated from this corpus
 * (`scripts/gen-nip-ad-tables.mjs`) so that particular drift cannot recur.
 *
 * See docs/nips/NIP-AD.md.
 */

/** Terminal states settle an obligation; non-terminal ones leave it open. */
export type DispositionState =
  | "completed"
  | "refused"
  | "responded"
  | "errored";

export const DISPOSITION_STATES: readonly DispositionState[] = [
  "completed",
  "refused",
  "responded",
  "errored",
];

/**
 * `completed` and `refused` settle the obligation. `responded` and `errored`
 * do not — which is what makes `errored → completed` and
 * `responded → completed` legal repairs rather than contradictions.
 */
export function isTerminalDisposition(state: DispositionState): boolean {
  return state === "completed" || state === "refused";
}

export function parseDispositionState(
  value: string | undefined,
): DispositionState | null {
  return value != null &&
    (DISPOSITION_STATES as readonly string[]).includes(value)
    ? (value as DispositionState)
    : null;
}

/**
 * Event kinds v1 accepts `["t","request"]` markers on.
 *
 * Centralized so every query builder and classifier derives its candidate
 * universe from one list. The CLI previously queried only kind 9 while the
 * desktop timeline carried several message and job kinds, so even identical
 * derivation logic could produce different accounting for one channel.
 *
 * Mirrors `REQUEST_KINDS` in the Rust module.
 */
export const REQUEST_KINDS: readonly number[] = [9];

export function isRequestKind(kind: number): boolean {
  return REQUEST_KINDS.includes(kind);
}

/** Why a marked event is malformed. Never an agent failure. */
export type InvalidRequestReason =
  | "missing_agent_target"
  | "malformed_agent_target"
  | "missing_channel"
  | "multiple_channels"
  | "target_not_mentioned"
  | "duplicate_agent_target"
  | "unsupported_kind";

/**
 * A well-formed request whose intent v1 cannot represent. Distinct from
 * invalid: an invalid request is a bug to fix, an unsupported one is a
 * feature to build. Both block a clean claim; only one implies fault.
 */
export type UnsupportedRequestReason = "multiple_agent_targets";

/**
 * Why an event is not a structurally valid kind:44300 disposition.
 *
 * Mirrors `InvalidDisposition` in the Rust module. Every member corresponds
 * to a rule in NIP-AD's "Event" and "Content" sections.
 */
export type InvalidDisposition =
  | "wrong_kind"
  | "request_id_cardinality"
  | "channel_cardinality"
  | "requester_cardinality"
  | "state_cardinality"
  | "malformed_request_id"
  | "malformed_requester"
  | "unknown_state"
  | "content_not_object"
  | "content_state_mismatch"
  | "missing_reason"
  | "reason_not_string"
  | "request_id_not_string"
  | "content_request_id_mismatch";

/**
 * Why a candidate disposition does not bind.
 *
 * Structural invalidity is flattened into this union rather than nested, so a
 * rejected-claims diagnostic names the rule that was broken.
 */
export type BindFailure =
  | InvalidDisposition
  | "request_mismatch"
  | "channel_mismatch"
  | "requester_mismatch"
  | "not_target_agent";

/** A structurally valid kind:44300 event, with its fields extracted once. */
export type CanonicalDisposition = {
  eventId: string;
  signer: string;
  createdAt: number;
  requestId: string;
  channelId: string;
  requesterPubkey: string;
  state: DispositionState;
  /** Required to be present; permitted to be empty. */
  reason: string;
};

/** Mirrors `KIND_AGENT_DISPOSITION` in buzz-core. */
export const KIND_AGENT_DISPOSITION = 44300;

/**
 * Something worth surfacing that does not change an outcome.
 *
 * Strictly diagnostic. An earlier version made every one of these force the
 * obligation out of its settled state and render as "disputed", which
 * defeated the point of categorizing them: a duplicate delivery and a genuine
 * contradiction produced identical, equally destructive results.
 */
export type HistoryWarning = "duplicate_terminal" | "ordered_after_terminal";

/** The one obligation a marked request creates in v1. */
export type Obligation = {
  requestId: string;
  channelId: string;
  requesterPubkey: string;
  targetAgentPubkey: string;
};

/** Total: every event lands in exactly one arm. No caller pre-check needed. */
export type RequestClass =
  | { kind: "not_request" }
  | { kind: "invalid"; reason: InvalidRequestReason }
  | { kind: "unsupported"; reason: UnsupportedRequestReason }
  | { kind: "valid"; obligation: Obligation };

/**
 * What actually became of an obligation.
 *
 * Separate from the raw latest observation on purpose. Deriving state as
 * "latest event wins" made `terminal` a lie: a late `errored` silently
 * reopened a settled obligation. Terminal claims are **absorbing** — once
 * something settles, a later weaker observation is a warning and cannot
 * reopen it.
 */
export type EffectiveOutcome =
  | { kind: "unanswered" }
  | { kind: "open"; state: DispositionState }
  | { kind: "settled"; state: DispositionState }
  | { kind: "disputed" };

export type DerivedObligation = {
  obligation: Obligation;
  /** What became of it. Every accounting and display surface uses this. */
  outcome: EffectiveOutcome;
  /**
   * The last bound state by `(created_at, id)`, regardless of absorption.
   * Retained for audit — it answers "what arrived last", a different
   * question from "what is true". Never decide doneness from it.
   */
  latestObservation: DispositionState | null;
  /** Reason from the disposition that determined the outcome. */
  reason: string;
  /** Diagnostics that do not change the outcome. */
  warnings: HistoryWarning[];
};

/** Minimal event shape this module needs — adapt from RelayEvent or a test. */
export type DispositionEventView = {
  id: string;
  pubkey: string;
  /**
   * Event kind. Required so request classification can reject kinds v1 does
   * not accept requests on — without it, two consumers querying different
   * kind sets produce different accounting and both look correct.
   */
  kind: number;
  created_at: number;
  content: string;
  tags: string[][];
};

function tagValues(event: DispositionEventView, key: string): string[] {
  return event.tags
    .filter((t) => t.length >= 2 && t[0] === key)
    .map((t) => t[1]);
}

function firstTag(
  event: DispositionEventView,
  key: string,
): string | undefined {
  return tagValues(event, key)[0];
}

function hasTagPair(
  event: DispositionEventView,
  key: string,
  value: string,
): boolean {
  return tagValues(event, key).includes(value);
}

/**
 * Canonical 64-character lowercase hex.
 *
 * Case is part of the contract, not formatting: a case-insensitive writer
 * paired with a case-sensitive reader silently produces dispositions nobody
 * binds. One canonical form, compared exactly, everywhere.
 */
export function isCanonicalHex64(value: string): boolean {
  return /^[0-9a-f]{64}$/.test(value);
}

/**
 * Whether an event carries the request marker. A marker check alone says
 * nothing about validity — prefer {@link classifyRequest}, which is total.
 */
export function isMarkedRequest(event: DispositionEventView): boolean {
  return hasTagPair(event, "t", "request");
}

/**
 * Classify any event. Total — no precondition, no caller-side marker check.
 *
 * Totality is the point. An earlier version required callers to check the
 * marker first, and the ACP harness grew its own looser predicate instead,
 * accepting uppercase targets, multi-target requests, and targets that were
 * never `p`-mentioned.
 */
export function classifyRequest(event: DispositionEventView): RequestClass {
  if (!isMarkedRequest(event)) {
    return { kind: "not_request" };
  }
  if (!isRequestKind(event.kind)) {
    return { kind: "invalid", reason: "unsupported_kind" };
  }

  const channels = tagValues(event, "h");
  if (channels.length === 0) {
    return { kind: "invalid", reason: "missing_channel" };
  }
  if (channels.length > 1) {
    return { kind: "invalid", reason: "multiple_channels" };
  }
  const channelId = channels[0];

  const agents = tagValues(event, "agent");
  if (agents.length === 0) {
    return { kind: "invalid", reason: "missing_agent_target" };
  }

  // **Malformed beats unsupported.** Every target is validated before
  // cardinality is judged, so a request naming one real agent and one garbage
  // value is a malformed event — not "a feature v1 cannot represent".
  // Returning unsupported first (as an earlier version did) filed client bugs
  // under "not anyone's fault".
  //
  // `p`-mention is checked in the same pass: `p` is what routes a message to
  // a principal, so an `agent` tag without it names a target that will never
  // be asked.
  for (const target of agents) {
    if (!isCanonicalHex64(target)) {
      return { kind: "invalid", reason: "malformed_agent_target" };
    }
    if (!hasTagPair(event, "p", target)) {
      return { kind: "invalid", reason: "target_not_mentioned" };
    }
  }

  const unique = [...new Set(agents)];
  if (unique.length > 1) {
    // Distinct, well-formed, reachable targets: a coherent intent v1 cannot
    // represent.
    return { kind: "unsupported", reason: "multiple_agent_targets" };
  }
  if (agents.length > 1) {
    // One target written twice — not canonical.
    return { kind: "invalid", reason: "duplicate_agent_target" };
  }
  const target = agents[0];

  return {
    kind: "valid",
    obligation: {
      requestId: event.id,
      channelId,
      requesterPubkey: event.pubkey,
      targetAgentPubkey: target,
    },
  };
}

/**
 * Whether `disposition` validly answers `obligation`.
 *
 * The target-agent clause is the cross-principal check: without it any
 * channel member — including a merely `p`-mentioned human — can close an
 * agent's obligation. The relay cannot enforce this (it would need to fetch
 * a second event mid-validation), so every consumer must, identically.
 */
/**
 * Validate an event against the **complete** NIP-AD structural contract.
 *
 * The single validity boundary, mirroring `validate_disposition_event` in the
 * Rust module (which relay ingest itself calls). It exists because the two
 * sides had drifted apart in the worst direction: the relay enforced kind,
 * exact tag cardinality, and a required string `reason`, while the
 * consumer-side binder checked none of them — it read the *first* matching
 * tag and ignored the rest. An event with two `e` tags, or no `reason`, or
 * not even of kind 44300, was rejected by the relay and simultaneously
 * accepted as a settling disposition here.
 */
export function validateDispositionEvent(
  event: DispositionEventView,
):
  | { valid: true; disposition: CanonicalDisposition }
  | { valid: false; reason: InvalidDisposition } {
  if (event.kind !== KIND_AGENT_DISPOSITION) {
    return { valid: false, reason: "wrong_kind" };
  }

  // Exactly one of each. `firstTag` semantics are deliberately not used here:
  // silently taking the first of several is how the validators diverged.
  const exactlyOne = (
    key: string,
    reason: InvalidDisposition,
  ):
    | { ok: true; value: string }
    | { ok: false; reason: InvalidDisposition } => {
    const values = tagValues(event, key);
    return values.length === 1
      ? { ok: true, value: values[0] }
      : { ok: false, reason };
  };

  const e = exactlyOne("e", "request_id_cardinality");
  if (!e.ok) return { valid: false, reason: e.reason };
  const h = exactlyOne("h", "channel_cardinality");
  if (!h.ok) return { valid: false, reason: h.reason };
  const p = exactlyOne("p", "requester_cardinality");
  if (!p.ok) return { valid: false, reason: p.reason };
  const d = exactlyOne("disposition", "state_cardinality");
  if (!d.ok) return { valid: false, reason: d.reason };

  if (!isCanonicalHex64(e.value)) {
    return { valid: false, reason: "malformed_request_id" };
  }
  if (!isCanonicalHex64(p.value)) {
    return { valid: false, reason: "malformed_requester" };
  }
  const state = parseDispositionState(d.value);
  if (state == null) {
    return { valid: false, reason: "unknown_state" };
  }

  let body: unknown;
  try {
    body = JSON.parse(event.content);
  } catch {
    return { valid: false, reason: "content_not_object" };
  }
  if (body == null || typeof body !== "object" || Array.isArray(body)) {
    return { valid: false, reason: "content_not_object" };
  }
  const record = body as Record<string, unknown>;
  if (record.disposition !== state) {
    return { valid: false, reason: "content_state_mismatch" };
  }
  if (!("reason" in record)) {
    return { valid: false, reason: "missing_reason" };
  }
  if (typeof record.reason !== "string") {
    return { valid: false, reason: "reason_not_string" };
  }
  if ("request_id" in record) {
    if (typeof record.request_id !== "string") {
      return { valid: false, reason: "request_id_not_string" };
    }
    if (record.request_id !== e.value) {
      return { valid: false, reason: "content_request_id_mismatch" };
    }
  }

  return {
    valid: true,
    disposition: {
      eventId: event.id,
      signer: event.pubkey,
      createdAt: event.created_at,
      requestId: e.value,
      channelId: h.value,
      requesterPubkey: p.value,
      state,
      reason: record.reason,
    },
  };
}

/**
 * Whether `disposition` validly answers `obligation`.
 *
 * Two stages: structural validity (above), then binding. The target-agent
 * clause is the cross-principal check — without it any channel member,
 * including a merely `p`-mentioned human, can close an agent's obligation.
 */
export function bindDisposition(
  obligation: Obligation,
  disposition: DispositionEventView,
):
  | { bound: true; state: DispositionState }
  | { bound: false; reason: BindFailure } {
  const validated = validateDispositionEvent(disposition);
  if (!validated.valid) {
    return { bound: false, reason: validated.reason };
  }
  return bindCanonical(obligation, validated.disposition);
}

/** Stage 2 alone, for callers that already validated. */
export function bindCanonical(
  obligation: Obligation,
  disposition: CanonicalDisposition,
):
  | { bound: true; state: DispositionState }
  | { bound: false; reason: BindFailure } {
  if (disposition.requestId !== obligation.requestId) {
    return { bound: false, reason: "request_mismatch" };
  }
  if (disposition.channelId !== obligation.channelId) {
    return { bound: false, reason: "channel_mismatch" };
  }
  if (disposition.requesterPubkey !== obligation.requesterPubkey) {
    return { bound: false, reason: "requester_mismatch" };
  }
  if (disposition.signer !== obligation.targetAgentPubkey) {
    return { bound: false, reason: "not_target_agent" };
  }
  return { bound: true, state: disposition.state };
}

function reasonOf(disposition: DispositionEventView): string {
  try {
    const body: unknown = JSON.parse(disposition.content);
    if (body != null && typeof body === "object") {
      const reason = (body as Record<string, unknown>).reason;
      if (typeof reason === "string") {
        return reason;
      }
    }
  } catch {
    // fall through
  }
  return "";
}

/** Ordinal (code-unit) comparison — matches Rust's byte-wise `str::cmp`
 *  exactly for the lowercase-hex ids this orders. Deliberately not
 *  `localeCompare`, whose collation is locale-dependent. */
function ordinalCompare(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

/**
 * Derive an obligation's state from candidate dispositions.
 *
 * Candidates are bound, deduplicated by event id (merged relay results can
 * legitimately carry the same event twice), then ordered by
 * `(created_at, id)`.
 *
 * **That ordering is deterministic, not causal.** `created_at` is
 * whole-second and publisher-supplied, so a later publication can carry an
 * earlier timestamp. `ordered_after_terminal` is named for what it can
 * actually establish — sort order — rather than for a write it cannot
 * observe.
 */
export function deriveObligation(
  obligation: Obligation,
  candidates: readonly DispositionEventView[],
): DerivedObligation {
  const bound: Array<{ event: DispositionEventView; state: DispositionState }> =
    [];
  const seen = new Set<string>();
  for (const candidate of candidates) {
    const result = bindDisposition(obligation, candidate);
    if (!result.bound || seen.has(candidate.id)) {
      continue;
    }
    seen.add(candidate.id);
    bound.push({ event: candidate, state: result.state });
  }

  bound.sort(
    (a, b) =>
      a.event.created_at - b.event.created_at ||
      ordinalCompare(a.event.id, b.event.id),
  );

  const latestObservation = bound[bound.length - 1]?.state ?? null;
  const hasCompleted = bound.some((b) => b.state === "completed");
  const hasRefused = bound.some((b) => b.state === "refused");
  const firstTerminalIdx = bound.findIndex((b) =>
    isTerminalDisposition(b.state),
  );

  // Terminal-absorbing: the first terminal claim decides, and a later
  // non-terminal observation is a warning rather than a reopening.
  let outcome: EffectiveOutcome;
  if (hasCompleted && hasRefused) {
    outcome = { kind: "disputed" };
  } else if (firstTerminalIdx !== -1) {
    outcome = { kind: "settled", state: bound[firstTerminalIdx].state };
  } else if (latestObservation != null) {
    outcome = { kind: "open", state: latestObservation };
  } else {
    outcome = { kind: "unanswered" };
  }

  const warnings: HistoryWarning[] = [];
  // "Duplicate" means the SAME terminal state twice. Counting any two
  // terminals would fire on `completed` + `refused`, which is a contradiction
  // (already `disputed`), not a duplicate.
  const duplicated = (["completed", "refused"] as const).some(
    (state) => bound.filter((b) => b.state === state).length > 1,
  );
  if (duplicated) {
    warnings.push("duplicate_terminal");
  }
  if (
    firstTerminalIdx !== -1 &&
    bound
      .slice(firstTerminalIdx + 1)
      .some((b) => !isTerminalDisposition(b.state))
  ) {
    warnings.push("ordered_after_terminal");
  }

  // The settling event's reason, not the latest event's — a stray write after
  // a refusal must not blank out why the agent refused.
  const reasonSource =
    firstTerminalIdx !== -1 && outcome.kind !== "disputed"
      ? bound[firstTerminalIdx]
      : bound[bound.length - 1];

  return {
    obligation,
    outcome,
    latestObservation,
    reason: reasonSource ? reasonOf(reasonSource.event) : "",
    warnings,
  };
}

export function isResolved(derived: DerivedObligation): boolean {
  return derived.outcome.kind === "settled";
}

export function isOpen(derived: DerivedObligation): boolean {
  return derived.outcome.kind === "open";
}

export function isUnanswered(derived: DerivedObligation): boolean {
  return derived.outcome.kind === "unanswered";
}

export function isDisputed(derived: DerivedObligation): boolean {
  return derived.outcome.kind === "disputed";
}

/**
 * How much of the record a caller actually examined.
 *
 * Both sides are required. A caller could paginate every request, fetch one
 * page of dispositions, see a `completed`, miss the later `refused`, and
 * truthfully set a one-sided flag while reporting a disputed obligation as
 * resolved.
 */
export type Coverage = {
  requestsComplete: boolean;
  dispositionsComplete: boolean;
};

export const PARTIAL_COVERAGE: Coverage = {
  requestsComplete: false,
  dispositionsComplete: false,
};

export const COMPLETE_COVERAGE: Coverage = {
  requestsComplete: true,
  dispositionsComplete: true,
};

export function isCoverageComplete(coverage: Coverage): boolean {
  return coverage.requestsComplete && coverage.dispositionsComplete;
}

/**
 * A stored disposition that named an obligation but did not bind to it.
 * Excluded from state; reported so spoof attempts are visible rather than
 * silently vanishing.
 */
export type RejectedClaim = {
  eventId: string;
  referencedRequest: string;
  signer: string;
  failure: BindFailure;
};

/** Accounting over a set of requests, with its coverage stated. */
export type Accounting = {
  settled: string[];
  open: string[];
  unanswered: string[];
  disputed: string[];
  invalidRequests: Array<{ requestId: string; reason: InvalidRequestReason }>;
  unsupportedRequests: Array<{
    requestId: string;
    reason: UnsupportedRequestReason;
  }>;
  rejectedClaims: RejectedClaim[];
  coverage: Coverage;
};

/**
 * Every request examined is settled, over provably complete coverage.
 *
 * `invalidRequests` and `unsupportedRequests` block this too, which is easy
 * to miss: neither is an agent failure, but both are marked events nobody can
 * ever answer, and a green check would hide the problem.
 */
export function allResolved(acc: Accounting): boolean {
  return (
    isCoverageComplete(acc.coverage) &&
    acc.open.length === 0 &&
    acc.unanswered.length === 0 &&
    acc.disputed.length === 0 &&
    acc.invalidRequests.length === 0 &&
    acc.unsupportedRequests.length === 0
  );
}

/** Requests examined, across every bucket. */
export function accountingTotal(acc: Accounting): number {
  return (
    acc.settled.length +
    acc.open.length +
    acc.unanswered.length +
    acc.disputed.length +
    acc.invalidRequests.length +
    acc.unsupportedRequests.length
  );
}

/**
 * Build accounting from candidate requests and dispositions.
 *
 * `requests` may contain any events — classification is total, and
 * non-requests are ignored. Dispositions are grouped by `e` tag once, so this
 * is O(requests + dispositions): any channel member can store structurally
 * valid but unbound dispositions, so the quadratic version was an avoidable
 * denial-of-service surface.
 */
export function account(
  requests: readonly DispositionEventView[],
  dispositions: readonly DispositionEventView[],
  coverage: Coverage,
): Accounting {
  const acc: Accounting = {
    settled: [],
    open: [],
    unanswered: [],
    disputed: [],
    invalidRequests: [],
    unsupportedRequests: [],
    rejectedClaims: [],
    coverage,
  };

  const byRequest = new Map<string, DispositionEventView[]>();
  for (const d of dispositions) {
    const e = firstTag(d, "e");
    if (e == null) {
      continue;
    }
    const list = byRequest.get(e);
    if (list) {
      list.push(d);
    } else {
      byRequest.set(e, [d]);
    }
  }

  for (const request of requests) {
    const classified = classifyRequest(request);
    switch (classified.kind) {
      case "not_request":
        break;
      case "invalid":
        acc.invalidRequests.push({
          requestId: request.id,
          reason: classified.reason,
        });
        break;
      case "unsupported":
        acc.unsupportedRequests.push({
          requestId: request.id,
          reason: classified.reason,
        });
        break;
      case "valid": {
        const candidates = byRequest.get(request.id) ?? [];
        const derived = deriveObligation(classified.obligation, candidates);
        const id = classified.obligation.requestId;
        switch (derived.outcome.kind) {
          case "settled":
            acc.settled.push(id);
            break;
          case "open":
            acc.open.push(id);
            break;
          case "unanswered":
            acc.unanswered.push(id);
            break;
          case "disputed":
            acc.disputed.push(id);
            break;
        }
        for (const candidate of candidates) {
          const result = bindDisposition(classified.obligation, candidate);
          if (!result.bound) {
            acc.rejectedClaims.push({
              eventId: candidate.id,
              referencedRequest: classified.obligation.requestId,
              signer: candidate.pubkey,
              failure: result.reason,
            });
          }
        }
        break;
      }
    }
  }

  return acc;
}
