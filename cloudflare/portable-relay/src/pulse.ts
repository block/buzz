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
 * conflict / diverged). The relay needs no special handling — ephemeral
 * fan-out carries responses like any other ephemeral event. */
export const KIND_BEACON_RESPONSE = 20701;

export const PULSE_ROLE_RENDEZVOUS = "rendezvous";
export const ADAPTER_ID = "portable-relay-cloudflare-v0.1";

/** The state one pulse witnesses; assembled by the node, signed as one event. */
export interface PulseState {
  stableNodeKey: string;
  nodeLabel: string;
  journal: { sequence: number; head: string | null };
  previous: string | null;
  checkpoints: Record<string, string>;
  agreements: Record<string, string>;
  governance: Record<string, "journal" | "bootstrap">;
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
        coherence: { governance: state.governance },
      }),
    },
    secret,
  );
}
