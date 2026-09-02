/**
 * Workstream card projection layer (S4 UI side).
 *
 * The Rust crate `buzz-core` (crates/buzz-core/src/cml_view.rs) projects a
 * full CML task into the privacy-safe, UI-ready `WorkstreamCard` serialized
 * here. This module is the TypeScript consumer of that wire contract: it
 * validates untrusted JSON, fails closed on anything unexpected, and never
 * invents data the backend did not send.
 *
 * Pure module: no React, no project imports, no network, no filesystem.
 */

/** Liveness as recomputed by the backend at observation time (lowercase). */
export type Liveness = "online" | "stale" | "offline";

export type WorkstreamPriority = "P0" | "P1" | "P2" | "P3";

export type WorkstreamStatus =
  | "proposed"
  | "planned"
  | "claimed"
  | "working"
  | "blocked"
  | "review"
  | "fixing"
  | "verified"
  | "integrated"
  | "shipped"
  | "cancelled"
  | "conflicted";

/**
 * Wire contract — exactly the 15 keys the Rust serde serializer emits.
 *
 * `head_short` and `host_id` are present-but-null when absent (serde emits
 * `null`, it does not skip the key). `liveness` is the value the backend
 * RECOMPUTED at `observed_at`, not the task's stored presence field.
 */
export interface WorkstreamCard {
  base_short: string;
  blocker_count: number;
  branch: string;
  head_short: string | null;
  host_id: string | null;
  live_claim: boolean;
  liveness: Liveness;
  objective: string;
  priority: WorkstreamPriority;
  repo: string;
  review_round: number;
  status: WorkstreamStatus;
  title: string;
  worktree_alias: string;
}

const LIVENESS_VALUES: readonly Liveness[] = ["online", "stale", "offline"];

const PRIORITY_VALUES: readonly WorkstreamPriority[] = ["P0", "P1", "P2", "P3"];

const STATUS_VALUES: readonly WorkstreamStatus[] = [
  "proposed",
  "planned",
  "claimed",
  "working",
  "blocked",
  "review",
  "fixing",
  "verified",
  "integrated",
  "shipped",
  "cancelled",
  "conflicted",
];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function requireString(card: Record<string, unknown>, field: string): string {
  const value = card[field];
  if (typeof value !== "string") {
    throw new Error(`WorkstreamCard: field "${field}" must be a string`);
  }
  return value;
}

function requireStringOrNull(
  card: Record<string, unknown>,
  field: string,
): string | null {
  const value = card[field];
  if (value !== null && typeof value !== "string") {
    throw new Error(
      `WorkstreamCard: field "${field}" must be a string or null`,
    );
  }
  return value;
}

function requireCount(card: Record<string, unknown>, field: string): number {
  const value = card[field];
  // The contract says unsigned integer; reject non-numbers, non-integers,
  // NaN, and negatives. Never coerce (a string "0" is a wrong primitive).
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0) {
    throw new Error(
      `WorkstreamCard: field "${field}" must be a non-negative integer`,
    );
  }
  return value;
}

function requireEnum<T extends string>(
  card: Record<string, unknown>,
  field: string,
  allowed: readonly T[],
): T {
  const value = card[field];
  if (typeof value !== "string" || !allowed.includes(value as T)) {
    throw new Error(
      `WorkstreamCard: field "${field}" has unknown value ${JSON.stringify(value)}`,
    );
  }
  return value as T;
}

/**
 * Validate and narrow untrusted JSON into a {@link WorkstreamCard}.
 *
 * Fails closed: a missing required field, wrong primitive type, unknown
 * liveness/status value, or negative counter throws — no defaults are ever
 * invented for missing data. A malformed card must never reach the board,
 * because rendering a half-parsed card could show a wrong liveness or claim
 * state (exactly the failure mode this projection exists to prevent).
 *
 * Note on liveness: this layer does NOT re-derive liveness. The stored CML
 * `runtime.presence` is signed at `updated_at`, so it is a frozen historical
 * value — echoing it on a board that renders later would show a long-dead
 * crashed worker as "online". The backend recomputes liveness at observation
 * time; we validate and trust that recomputed value, and nothing else.
 */
export function parseWorkstreamCard(raw: unknown): WorkstreamCard {
  if (!isRecord(raw)) {
    throw new Error("WorkstreamCard: expected a card object");
  }
  if (typeof raw.live_claim !== "boolean") {
    throw new Error('WorkstreamCard: field "live_claim" must be a boolean');
  }
  return {
    base_short: requireString(raw, "base_short"),
    blocker_count: requireCount(raw, "blocker_count"),
    branch: requireString(raw, "branch"),
    head_short: requireStringOrNull(raw, "head_short"),
    host_id: requireStringOrNull(raw, "host_id"),
    live_claim: raw.live_claim,
    liveness: requireEnum(raw, "liveness", LIVENESS_VALUES),
    objective: requireString(raw, "objective"),
    priority: requireEnum(raw, "priority", PRIORITY_VALUES),
    repo: requireString(raw, "repo"),
    review_round: requireCount(raw, "review_round"),
    status: requireEnum(raw, "status", STATUS_VALUES),
    title: requireString(raw, "title"),
    worktree_alias: requireString(raw, "worktree_alias"),
  };
}

/**
 * Placeholder shown when `head_short` is null. A SHA we do not have must
 * render as visibly-missing — fabricating or deriving one from `base_short`
 * (or any other field) would put a fake commit identity on the board.
 */
export const NO_HEAD_PLACEHOLDER = "(no head)";

/**
 * Render the head SHA short form, or {@link NO_HEAD_PLACEHOLDER} when the
 * card has no head. Never derives a SHA from any other field.
 */
export function formatHeadShort(card: WorkstreamCard): string {
  return card.head_short ?? NO_HEAD_PLACEHOLDER;
}

/**
 * True only when the card should render as a *live claim*: liveness is
 * "online" AND the claim lease is held (`live_claim === true`).
 *
 * Both conditions are required because lease expiry is INDEPENDENT of
 * heartbeat freshness: a worker can have a fresh heartbeat while its claim
 * lease has expired. Rendering that card as "someone is on it" would show a
 * live claim nobody holds, inviting duplicate claims — so an online
 * heartbeat with an expired lease must not display as a live claim.
 */
export function isDisplayableLiveClaim(card: WorkstreamCard): boolean {
  return card.liveness === "online" && card.live_claim === true;
}

// --- S4 privacy guard -------------------------------------------------------
//
// Public UI and task state may never carry absolute filesystem paths, raw IP
// addresses, or full pubkeys/SHAs. Only `worktree_alias` and the pseudonymous
// `host_id` identify where work happens. The Rust projection is designed not
// to emit these, but the TS layer enforces the invariant independently so a
// backend regression cannot silently leak a user's machine identity.

/** Absolute POSIX path ("/Users/...") or Windows drive path ("C:\..." / "C:/..."). */
const ABSOLUTE_PATH_PATTERN = /^(?:\/|[A-Za-z]:[\\/])/;

/** Four dot-separated decimal octets, e.g. "192.168.1.44". */
const IPV4_PATTERN = /(?:^|[^0-9.])(?:\d{1,3}\.){3}\d{1,3}(?:[^0-9.]|$)/;

/** A full 40-character hex string (git SHA-1 / nostr pubkey material). */
const FULL_SHA_PATTERN = /(?:^|[^0-9a-fA-F])[0-9a-fA-F]{40}(?:[^0-9a-fA-F]|$)/;

/**
 * Throw if any string field of the card leaks sensitive machine identity:
 * an absolute filesystem path, a raw IPv4 address, or a full 40-char hex SHA.
 * Silent-passing here would mean a privacy violation shipped to public UI.
 */
export function assertNoSensitiveLeak(card: WorkstreamCard): void {
  for (const [field, value] of Object.entries(card)) {
    if (typeof value !== "string") {
      continue;
    }
    if (ABSOLUTE_PATH_PATTERN.test(value)) {
      throw new Error(
        `WorkstreamCard: field "${field}" leaks an absolute filesystem path`,
      );
    }
    if (IPV4_PATTERN.test(value)) {
      throw new Error(
        `WorkstreamCard: field "${field}" leaks a raw IPv4 address`,
      );
    }
    if (FULL_SHA_PATTERN.test(value)) {
      throw new Error(
        `WorkstreamCard: field "${field}" leaks a full 40-character hex SHA`,
      );
    }
  }
}
