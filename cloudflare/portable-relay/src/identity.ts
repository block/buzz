// Portable identity verification and policy for the Cloudflare adapter.
//
// Implements the portable-relay-identity-v0.1 mapping: NIP-42 scoped to one
// WebSocket, payload-bound NIP-98 scoped to one HTTP request, direct writes
// bound to the authenticated author with the NIP-59 gift-wrap exception, and
// result-level disclosure applied identically to query, count, historical,
// and live delivery. Denial codes are the profile's stable wire tokens;
// human-readable messages remain informative only.

import { verifyEvent, type Event } from "nostr-tools";

/** Freshness window shared by NIP-42 and NIP-98 proofs (buzz-auth parity). */
export const TIMESTAMP_TOLERANCE_SECS = 60;
/** Consumed-proof retention; anything older is unusable by freshness alone. */
export const PROOF_RETENTION_SECS = TIMESTAMP_TOLERANCE_SECS * 10;

export const KIND_AUTH = 22242;
export const KIND_HTTP_AUTH = 27235;
export const KIND_GIFT_WRAP = 1059;

// Kind policy sets ported from buzz-core/src/kind.rs — keep in sync.
const AUTHOR_ONLY_KINDS = new Set([30_300, 30_350]);
const P_GATED_KINDS = new Set([24_200, 44_100, 44_101, 1_059, 30_622, 44_200]);

const HEX_32_BYTES = /^[0-9a-f]{64}$/;

/** Stable denial tokens from portable-relay-identity-v0.1. */
export type DenialCode =
  | "authentication_required"
  | "invalid_evidence"
  | "evidence_expired"
  | "audience_mismatch"
  | "replay_detected"
  | "author_mismatch"
  | "peer_unbound"
  | "source_mismatch"
  | "scope_denied"
  | "event_disclosure_denied";

export type VerificationResult =
  | { ok: true; pubkey: string; proofId: string }
  | { ok: false; code: DenialCode };

export function httpStatusForDenial(code: DenialCode): number {
  switch (code) {
    case "authentication_required":
    case "invalid_evidence":
    case "evidence_expired":
    case "audience_mismatch":
    case "replay_detected":
      return 401;
    default:
      return 403;
  }
}

function verifySafely(event: Event): boolean {
  try {
    return verifyEvent(event);
  } catch {
    return false;
  }
}

function tagValue(event: Event, name: string): string | undefined {
  return event.tags.find((tag) => tag[0] === name)?.[1];
}

/**
 * Compares the authority of two relay/request URLs. NIP-42 relay tags may
 * spell the ws/wss scheme differently from the adapter-derived audience, so
 * only host and port carry the binding.
 */
function sameAuthority(left: string, right: string): boolean {
  try {
    return (
      new URL(left).host.toLowerCase() === new URL(right).host.toLowerCase()
    );
  } catch {
    return false;
  }
}

function withinTolerance(createdAt: number, nowSecs: number): boolean {
  return Math.abs(nowSecs - createdAt) <= TIMESTAMP_TOLERANCE_SECS;
}

/** Verifies a NIP-42 challenge response at an explicit evaluation time. */
export function verifyNip42At(
  event: Event,
  expectedChallenge: string,
  audience: string,
  nowSecs: number,
): VerificationResult {
  if (event.kind !== KIND_AUTH || !verifySafely(event)) {
    return { ok: false, code: "invalid_evidence" };
  }
  if (tagValue(event, "challenge") !== expectedChallenge) {
    return { ok: false, code: "audience_mismatch" };
  }
  const relay = tagValue(event, "relay");
  if (relay === undefined || !sameAuthority(relay, audience)) {
    return { ok: false, code: "audience_mismatch" };
  }
  if (!withinTolerance(event.created_at, nowSecs)) {
    return { ok: false, code: "evidence_expired" };
  }
  return { ok: true, pubkey: event.pubkey, proofId: event.id };
}

/** Request facts a NIP-98 proof must cover. */
export interface Nip98Expectation {
  url: string;
  method: string;
  payloadSha256Hex: string;
}

/** Verifies a payload-bound NIP-98 proof at an explicit evaluation time. */
export function verifyNip98At(
  event: Event,
  expected: Nip98Expectation,
  nowSecs: number,
): VerificationResult {
  if (event.kind !== KIND_HTTP_AUTH || !verifySafely(event)) {
    return { ok: false, code: "invalid_evidence" };
  }
  const url = tagValue(event, "u");
  if (url === undefined || !urlsEqual(url, expected.url)) {
    return { ok: false, code: "audience_mismatch" };
  }
  if (tagValue(event, "method") !== expected.method) {
    return { ok: false, code: "audience_mismatch" };
  }
  if (tagValue(event, "payload") !== expected.payloadSha256Hex) {
    return { ok: false, code: "invalid_evidence" };
  }
  if (!withinTolerance(event.created_at, nowSecs)) {
    return { ok: false, code: "evidence_expired" };
  }
  return { ok: true, pubkey: event.pubkey, proofId: event.id };
}

function urlsEqual(left: string, right: string): boolean {
  try {
    const a = new URL(left);
    const b = new URL(right);
    return (
      a.origin.toLowerCase() === b.origin.toLowerCase() &&
      a.pathname === b.pathname &&
      a.search === b.search
    );
  } catch {
    return false;
  }
}

/**
 * Direct-append policy: authentication events never enter the journal, and
 * the caller must be the event author except for NIP-59 gift-wrap author
 * indirection with at least one well-formed recipient.
 */
export function authorizeDirect(
  principalPubkey: string,
  event: Event,
): DenialCode | null {
  if (event.kind === KIND_AUTH || event.kind === KIND_HTTP_AUTH) {
    return "scope_denied";
  }
  if (event.pubkey === principalPubkey) {
    return null;
  }
  if (
    event.kind === KIND_GIFT_WRAP &&
    event.tags.some((tag) => tag[0] === "p" && HEX_32_BYTES.test(tag[1] ?? ""))
  ) {
    return null;
  }
  return "author_mismatch";
}

/**
 * Result-level disclosure applied to query, count, historical subscription,
 * and live delivery. Author-only kinds disclose only to their author;
 * `#p`-gated kinds disclose only to tagged recipients.
 */
export function eventVisibleToReader(
  readerPubkey: string,
  event: Event,
): boolean {
  if (AUTHOR_ONLY_KINDS.has(event.kind) && event.pubkey !== readerPubkey) {
    return false;
  }
  if (
    P_GATED_KINDS.has(event.kind) &&
    !event.tags.some((tag) => tag[0] === "p" && tag[1] === readerPubkey)
  ) {
    return false;
  }
  return true;
}
