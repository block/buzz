// Beacon pulse — the node's signed witness statement.
//
// A pulse (kind 20700, ephemeral range) is the custodian's own declaration
// of the state it currently witnesses: journal head, replication
// checkpoints, and the agreement heads it applies. It is a signal about
// state, not state itself: never journaled, never replicated, and it
// asserts no canonicality — recognition emerges when peers observe
// compatible heads. Pulses are signed with the node's witness key
// (BUZZ_NODE_SECRET), a distinct identity from the owner: the node speaks
// for what it holds, never for its operator.

import { finalizeEvent, getPublicKey, type Event } from "nostr-tools";

/** PROVISIONAL kind pending upstream registry assignment (buzz-core/src/kind.rs).
 * The ephemeral counterpart of the kind-30700 sync declaration: 30700 is
 * the durable agreement, 20700 the ephemeral witness of now. */
export const KIND_BEACON_PULSE = 20700;

/** PROVISIONAL kind for peer responses to a pulse (recognize / advanced /
 * conflict / diverged). Responses remain ephemeral, while the relay
 * validates and briefly tallies them for its next pulse. */
export const KIND_BEACON_RESPONSE = 20701;

export const PULSE_ROLE_RENDEZVOUS = "rendezvous";
export const ADAPTER_ID = "portable-relay-cloudflare-v0.1";
export const DEFAULT_RECOGNITION_WINDOW_SECS = 300;

export const BEACON_STANCES = [
  "recognize",
  "advanced",
  "conflict",
  "diverged",
  "unsatisfied",
] as const;

export type BeaconStance = (typeof BEACON_STANCES)[number];

export interface PulseSessions {
  count: number;
  principals: string[];
}

export interface PulseRecognition {
  head: string;
  pulse: string;
  responses: Record<string, BeaconStance>;
  window_secs: number;
}

/** The state one pulse witnesses; assembled by the node, signed as one event. */
export interface PulseState {
  stableNodeKey: string;
  nodeLabel: string;
  journal: { sequence: number; head: string | null };
  previous: string | null;
  checkpoints: Record<string, string>;
  agreements: Record<string, string>;
  governance: Record<string, "journal" | "bootstrap">;
  sessions?: PulseSessions;
  recognition?: PulseRecognition;
}

const HEX_32_BYTE_SECRET = /^[0-9a-f]{64}$/i;

/**
 * Parses the node witness secret. Anything but exactly 32 hex-encoded bytes
 * yields null: the pulse capability is absent rather than misconfigured.
 */
export function witnessSecretFromEnv(
  raw: string | undefined,
): Uint8Array | null {
  if (raw === undefined || !HEX_32_BYTE_SECRET.test(raw)) {
    return null;
  }
  const bytes = new Uint8Array(32);
  for (let index = 0; index < bytes.length; index += 1) {
    bytes[index] = Number.parseInt(raw.slice(index * 2, index * 2 + 2), 16);
  }
  return bytes;
}

/** The node's witness identity, or null when the capability is absent. */
export function witnessPubkeyFromEnv(raw: string | undefined): string | null {
  const secret = witnessSecretFromEnv(raw);
  if (secret === null) {
    return null;
  }
  try {
    return getPublicKey(secret);
  } catch {
    return null;
  }
}

/**
 * Builds and signs one pulse. The content is the witness statement; tags
 * carry only routing hints (`n` for the node label, `role` for the node's
 * declared function) so tooling can filter without parsing content.
 */
export function buildPulseEvent(
  state: PulseState,
  secret: Uint8Array,
  nowSecs: number,
): Event {
  const tags: string[][] = [["role", PULSE_ROLE_RENDEZVOUS]];
  if (state.nodeLabel !== "") {
    tags.unshift(["n", state.nodeLabel]);
  }
  return finalizeEvent(
    {
      kind: KIND_BEACON_PULSE,
      created_at: nowSecs,
      tags,
      content: JSON.stringify({
        node: state.stableNodeKey,
        label: state.nodeLabel,
        adapter: ADAPTER_ID,
        journal: state.journal,
        previous: state.previous,
        checkpoints: state.checkpoints,
        agreements: state.agreements,
        coherence: {
          governance: state.governance,
          ...(state.sessions === undefined ? {} : { sessions: state.sessions }),
          ...(state.recognition === undefined
            ? {}
            : { recognition: state.recognition }),
        },
      }),
    },
    secret,
  );
}

export interface ExpectedBeaconResponse {
  pulseId: string;
  pulseHead: string;
  pulseCreatedAt: number;
  witnessPubkey: string;
  windowSecs?: number;
}

export interface ParsedBeaconResponse {
  stance: BeaconStance;
  mine: { sequence: number; head: string | null };
  observed: Record<string, unknown>;
}

/**
 * Validates the protocol shape and freshness of one kind-20701 answer to an
 * active pulse. Signature and authenticated-author checks remain the relay
 * ingest layer's responsibility; this function binds the response to the
 * exact pulse, witness, and journal head whose roll is still open.
 */
export function parseBeaconResponse(
  event: Event,
  expected: ExpectedBeaconResponse,
  nowSecs: number,
): ParsedBeaconResponse | null {
  if (event.kind !== KIND_BEACON_RESPONSE) {
    return null;
  }
  if (
    !event.tags.some((tag) => tag[0] === "e" && tag[1] === expected.pulseId) ||
    !event.tags.some(
      (tag) => tag[0] === "p" && tag[1] === expected.witnessPubkey,
    )
  ) {
    return null;
  }
  const windowSecs = expected.windowSecs ?? DEFAULT_RECOGNITION_WINDOW_SECS;
  if (
    event.created_at < expected.pulseCreatedAt ||
    event.created_at > expected.pulseCreatedAt + windowSecs ||
    nowSecs > expected.pulseCreatedAt + windowSecs
  ) {
    return null;
  }
  let content: unknown;
  try {
    content = JSON.parse(event.content);
  } catch {
    return null;
  }
  if (!isRecord(content) || content.head !== expected.pulseHead) {
    return null;
  }
  if (
    typeof content.stance !== "string" ||
    !BEACON_STANCES.some((stance) => stance === content.stance)
  ) {
    return null;
  }
  if (!isRecord(content.mine)) {
    return null;
  }
  const sequence = content.mine.sequence;
  const head = content.mine.head;
  if (
    typeof sequence !== "number" ||
    !Number.isSafeInteger(sequence) ||
    sequence < 0 ||
    !(
      head === null ||
      (typeof head === "string" && /^[0-9a-f]{64}$/.test(head))
    )
  ) {
    return null;
  }
  const observed = content.observed;
  if (
    !isRecord(observed) ||
    !validObserved(content.stance as BeaconStance, observed)
  ) {
    return null;
  }
  return {
    stance: content.stance as BeaconStance,
    mine: { sequence, head },
    observed,
  };
}

function validObserved(
  stance: BeaconStance,
  observed: Record<string, unknown>,
): boolean {
  switch (stance) {
    case "recognize":
      return true;
    case "advanced":
      return (
        typeof observed.since === "number" &&
        Number.isSafeInteger(observed.since) &&
        observed.since >= 0
      );
    case "conflict":
      return (
        typeof observed.claim === "string" &&
        observed.claim !== "" &&
        typeof observed.mine === "string" &&
        observed.mine !== ""
      );
    case "diverged":
      return (
        (observed.measure === "head-unknown" ||
          observed.measure === "agreements") &&
        typeof observed.detail === "string" &&
        observed.detail !== ""
      );
    case "unsatisfied":
      return (
        typeof observed.agreement === "string" &&
        observed.agreement !== "" &&
        typeof observed.reason === "string" &&
        observed.reason !== ""
      );
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
