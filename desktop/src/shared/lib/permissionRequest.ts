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
 * - All untrusted display strings are size-bounded (≤ 200 chars) and
 *   HTML-escaped by React at render time.
 */

// ── Types ─────────────────────────────────────────────────────────────────────

/**
 * Pending sentinel — the card is actionable.
 *
 * `requestNonce` and `expiresAt` are trusted as unsigned ints from the harness.
 * `labels` values are untrusted display strings (capped at 200 chars).
 */
export type PermissionRequestPending = {
  v: 1;
  state: "pending";
  requestNonce: string;
  sessionId: string | null;
  turnId: string | null;
  expiresAt: number;
  /** Opaque option IDs. Size-bounded: ≤ 10. */
  optionIds: string[];
  /** Harness-provided display labels keyed by optionId. Each ≤ 200 chars. */
  labels: Record<string, string>;
  /** True when an `allow_always` option is present (D5 durable-rule disclosure). */
  hasDurableRule: boolean;
  /**
   * Human-readable durable-rule disclosure note. Non-null only when
   * `hasDurableRule === true`. E.g. "Includes an 'Always allow' option —
   * creates a machine-wide durable rule in Codex."
   */
  durableRuleNote: string | null;
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
  hasDurableRule: boolean;
  durableRuleNote: string | null;
  /** Outcome of the permission request. */
  outcome: "applied" | "timed_out" | "cancelled" | "rejected";
  /** Non-null only when outcome === "applied". */
  chosenOptionId: string | null;
};

export type PermissionRequestPayload =
  | PermissionRequestPending
  | PermissionRequestResolved;

// ── Constants ─────────────────────────────────────────────────────────────────

/** Maximum character length for any untrusted display string in the sentinel. */
const MAX_LABEL_CHARS = 200;

/** Maximum number of option IDs in a sentinel (PERMISSION_OPTIONS_MAX). */
const MAX_OPTION_IDS = 10;

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
  return typeof v === "string" && v.length <= MAX_LABEL_CHARS;
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
  if (typeof p.requestNonce !== "string" || p.requestNonce.length === 0) {
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
  if (
    !Array.isArray(p.optionIds) ||
    p.optionIds.length === 0 ||
    p.optionIds.length > MAX_OPTION_IDS ||
    !p.optionIds.every((id) => typeof id === "string" && id.length > 0)
  ) {
    return false;
  }
  if (!isLabelsRecord(p.labels)) return false;
  // Every advertised optionId must have a label entry
  if (
    !(p.optionIds as string[]).every(
      (id) => typeof (p.labels as Record<string, unknown>)[id] === "string",
    )
  ) {
    return false;
  }
  if (typeof p.hasDurableRule !== "boolean") return false;
  if (!isNullableString(p.durableRuleNote)) return false;

  if (p.state === "pending") {
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
    // chosenOptionId: non-null ⟺ outcome === "applied"
    if (p.outcome === "applied") {
      if (typeof p.chosenOptionId !== "string" || p.chosenOptionId.length === 0)
        return false;
    } else {
      if (p.chosenOptionId !== null) return false;
    }
    return true;
  }

  return false;
}
