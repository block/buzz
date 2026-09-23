/**
 * Utilities for extracting and parsing the permission-request sentinel that
 * `buzz-acp` publishes as a kind:9 reply into the triggering thread when an
 * `ask`-policy permission request is admitted.
 *
 * Wire format (versioned discriminated union, schema v1 — frozen at event
 * b31c716e):
 *
 * The harness serialises a bare JSON object as the kind:9 event content:
 *
 *   {"v":1,"state":"pending","requestNonce":"…", …}
 *
 * Desktop identifies a sentinel by `"v":1` in the top-level JSON object.
 * Non-JSON content and JSON objects without `"v":1` are left untouched.
 * There is no fenced wire format — non-sentinel kind:9s must NOT be modified.
 *
 * Security invariants:
 * - `agentPubkey` and `channelId` are derived from the SIGNED EVENT ENVELOPE,
 *   never from sentinel JSON.
 * - `optionId` values are opaque — treated as arbitrary strings; never
 *   interpreted as ACP kinds by the renderer.
 * - Labels come from `labels[optionId]` — harness-provided display strings,
 *   not raw ACP kind names.
 * - All untrusted display strings are size-bounded (≤ 200 UTF-8 bytes) and
 *   HTML-escaped by React at render time.
 */

// ── Types ─────────────────────────────────────────────────────────────────────

/**
 * Pending sentinel — the card is actionable.
 *
 * `requestNonce` and `expiresAt` are trusted as unsigned ints from the harness.
 * `labels` values are untrusted display strings (capped at 200 UTF-8 bytes).
 */
export type PermissionRequestPending = {
  v: 1;
  state: "pending";
  requestNonce: string;
  sessionId: string | null;
  turnId: string | null;
  expiresAt: number;
  /**
   * Opaque option IDs. Exactly two — the ruled `allow_once` and `reject_once`
   * actions, in that order. The harness never forwards a third option (e.g.
   * `allow_always`), so any other cardinality is a malformed sentinel.
   */
  optionIds: string[];
  /** Harness-provided display labels keyed by optionId. Each ≤ 200 UTF-8 bytes. */
  labels: Record<string, string>;
  /**
   * Human-readable description of the requested operation, sourced from the
   * ACP `session/request_permission` message via `description_from_request_permission`
   * in `crates/buzz-acp/src/acp.rs`. Tries `params.title`,
   * `params.subject.toolCall.title`, `params.toolCall.title`,
   * `params.toolCall.rawInput.command`, and `params._meta.codex.params.reason` in order.
   * Truncated producer-side to ≤ 200 UTF-8 bytes. `null` when no path yields a
   * non-empty string.
   *
   * Display-only — treated as an untrusted string and rendered as text (HTML-
   * escaped by React). The sentinel is valid when this field is absent or null.
   */
  description?: string | null;
};

/**
 * Resolved sentinel — the card is non-actionable (archived state).
 *
 * Published by the harness as a kind-40003 edit signed by the original agent.
 * `originalEventId` is the kind-9 event ID — correlates the edit to the card.
 */
export type PermissionRequestResolved = {
  v: 1;
  state: "resolved";
  requestNonce: string;
  originalEventId: string;
  sessionId: string | null;
  turnId: string | null;
  expiresAt: number;
  optionIds: string[];
  labels: Record<string, string>;
  /** Outcome of the permission request. */
  outcome: "applied" | "timed_out" | "cancelled" | "rejected";
  /** Non-null only when outcome === "applied". */
  chosenOptionId: string | null;
  /**
   * Human-readable description of the requested operation — same value as in
   * the corresponding pending sentinel. `null` or absent when no subject was
   * provided. Rendered on the resolved card for context.
   */
  description?: string | null;
};

export type PermissionRequestPayload =
  | PermissionRequestPending
  | PermissionRequestResolved;

// ── Constants ─────────────────────────────────────────────────────────────────

/**
 * Frozen sentinel byte bounds — shared verbatim with the harness producer
 * (`SENTINEL_STRING_MAX_BYTES` / `SENTINEL_CONTENT_MAX_BYTES` in
 * `crates/buzz-acp/src/acp.rs`).
 *
 * Both the values AND the unit — UTF-8 bytes — must match the producer. The
 * prior split (harness truncated labels by Rust `char` scalars while this parser
 * bounded by JavaScript `.length` UTF-16 code units) let a producer-valid
 * multibyte label be rejected here, publishing a card the desktop renders as raw
 * JSON until timeout. Measuring in bytes on both sides closes that gap.
 *
 * `MAX_STRING_BYTES` bounds every untrusted string leaf: labels, each
 * `optionId`, `requestNonce`, `sessionId`, `turnId`, and `chosenOptionId`.
 * `MAX_CONTENT_BYTES` bounds the total serialized sentinel content.
 */
const MAX_STRING_BYTES = 200;
const MAX_CONTENT_BYTES = 4096;

/** UTF-8 byte length of a string — the shared measurement unit. */
const UTF8 = new TextEncoder();
function byteLength(s: string): number {
  return UTF8.encode(s).length;
}

/**
 * Exact number of option IDs in a sentinel. The card is a two-action contract:
 * the ruled `allow_once` and `reject_once` options and nothing else. The
 * harness enforces this on the producer side (`select_card_actions`); the
 * parser rejects any other cardinality so the two sides can never diverge.
 */
const OPTION_IDS_COUNT = 2;

/** Regex for a valid 64-character lowercase hex Nostr event ID. */
const HEX64_RE = /^[0-9a-f]{64}$/;

/** The four valid outcome strings. */
const VALID_OUTCOMES = new Set([
  "applied",
  "timed_out",
  "cancelled",
  "rejected",
]);

// ── Extractor ─────────────────────────────────────────────────────────────────

/**
 * Extract the `PermissionRequestPayload` from a kind:9 event content string,
 * if present.
 *
 * The harness signs bare JSON as the event content — no fence wrapper. Desktop
 * identifies sentinels by `"v":1` at the top level. Non-JSON content and JSON
 * objects that do not carry `"v":1` are returned as `null`; `MessageRow` renders
 * them as ordinary markdown.
 *
 * Returns `null` when:
 * - the content is not valid JSON
 * - the parsed value is not a sentinel object (missing `v:1`)
 * - the parsed value does not match the expected shape or invariants
 *
 * Never throws — all errors are swallowed so this is safe in the render path.
 */
export function extractPermissionRequest(
  content: string,
): PermissionRequestPayload | null {
  // Total-content byte bound — the single size gate mirrored by the harness
  // (`SENTINEL_CONTENT_MAX_BYTES`). Measured against the RAW content, not the
  // trimmed value, so the bound matches the producer's complete serialized
  // output byte-for-byte; gating trimmed content would let whitespace padding
  // smuggle signed content past the frozen boundary. Reject before parsing so
  // an oversized signed payload can never allocate an outsized DOM/control value.
  if (byteLength(content) > MAX_CONTENT_BYTES) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(content.trim());
  } catch {
    return null;
  }
  return isPermissionRequestPayload(parsed) ? parsed : null;
}

/**
 * Returns `true` when the kind:9 content is a permission-request sentinel.
 * Used by `MessageRow` to decide whether to suppress markdown rendering.
 *
 * When `extractPermissionRequest` returns a non-null value the content IS the
 * sentinel; the entire string is consumed by the card. Non-sentinel kind:9s are
 * rendered as ordinary markdown, unchanged.
 */
export function isPermissionRequestSentinel(content: string): boolean {
  return extractPermissionRequest(content) !== null;
}

// ── Type guards ────────────────────────────────────────────────────────────────

function isSafeString(v: unknown): v is string {
  return typeof v === "string" && byteLength(v) <= MAX_STRING_BYTES;
}

function isNullableString(v: unknown): v is string | null {
  return v === null || isSafeString(v);
}

function isLabelsRecord(v: unknown): v is Record<string, string> {
  if (typeof v !== "object" || v === null || Array.isArray(v)) return false;
  return Object.values(v as Record<string, unknown>).every(isSafeString);
}

function isPermissionRequestPayload(v: unknown): v is PermissionRequestPayload {
  if (typeof v !== "object" || v === null || Array.isArray(v)) return false;
  const p = v as Record<string, unknown>;
  if (p.v !== 1) return false;

  // Shared fields present in both states
  if (
    typeof p.requestNonce !== "string" ||
    p.requestNonce.length === 0 ||
    byteLength(p.requestNonce) > MAX_STRING_BYTES
  ) {
    return false;
  }
  if (!isNullableString(p.sessionId)) return false;
  if (!isNullableString(p.turnId)) return false;
  // expiresAt must be an integer (no fractional seconds, no negative values)
  if (
    typeof p.expiresAt !== "number" ||
    !Number.isFinite(p.expiresAt) ||
    !Number.isInteger(p.expiresAt) ||
    p.expiresAt < 0
  ) {
    return false;
  }
  // optionIds: EXACTLY two bounded, non-empty, unique opaque strings.
  if (
    !Array.isArray(p.optionIds) ||
    p.optionIds.length !== OPTION_IDS_COUNT ||
    !p.optionIds.every(
      (id) =>
        typeof id === "string" &&
        id.length > 0 &&
        byteLength(id) <= MAX_STRING_BYTES,
    )
  ) {
    return false;
  }
  if (
    new Set(p.optionIds as string[]).size !== (p.optionIds as string[]).length
  ) {
    return false;
  }
  if (!isLabelsRecord(p.labels)) return false;
  // labels must have exactly one entry per advertised optionId — no extra keys.
  const optionIds = p.optionIds as string[];
  const labelKeys = Object.keys(p.labels as Record<string, unknown>);
  if (labelKeys.length !== optionIds.length) return false;
  if (
    !optionIds.every(
      (id) => typeof (p.labels as Record<string, unknown>)[id] === "string",
    )
  ) {
    return false;
  }

  if (p.state === "pending") {
    // description: optional field — absent/null or a bounded string are all valid.
    if (
      "description" in p &&
      p.description !== null &&
      p.description !== undefined &&
      (typeof p.description !== "string" ||
        byteLength(p.description) > MAX_STRING_BYTES)
    ) {
      return false;
    }
    return true;
  }

  if (p.state === "resolved") {
    // originalEventId: 64-char lowercase hex string
    if (
      typeof p.originalEventId !== "string" ||
      !HEX64_RE.test(p.originalEventId)
    ) {
      return false;
    }
    // outcome: exactly one of the four literals
    if (!VALID_OUTCOMES.has(p.outcome as string)) return false;
    // chosenOptionId: non-null ⟺ outcome === "applied", bounded, and must be
    // one of the advertised optionIds.
    if (p.outcome === "applied") {
      if (
        typeof p.chosenOptionId !== "string" ||
        p.chosenOptionId.length === 0 ||
        byteLength(p.chosenOptionId) > MAX_STRING_BYTES ||
        !optionIds.includes(p.chosenOptionId)
      ) {
        return false;
      }
    } else {
      if (p.chosenOptionId !== null) return false;
    }
    // description: optional field — same rules as pending.
    if (
      "description" in p &&
      p.description !== null &&
      p.description !== undefined &&
      (typeof p.description !== "string" ||
        byteLength(p.description) > MAX_STRING_BYTES)
    ) {
      return false;
    }
    return true;
  }

  return false;
}
