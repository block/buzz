import { signRelayEvent } from "@/shared/api/tauri";
import {
  buildSpawnerCredentialUpdate,
  buildSpawnerPromptUpdate,
} from "@/shared/api/tauriSpawner";
import type { SpawnerPromptMaterial } from "@/shared/api/tauriSpawner";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_SPAWNER_AGENT_SPEC,
  KIND_SPAWNER_AGENT_STATUS,
  KIND_SPAWNER_ANNOUNCEMENT,
  KIND_SPAWNER_ATTESTATION,
} from "@/shared/constants/kinds";
import { relayClient } from "./relayClient";

/**
 * Lookback for the attestation subscription.
 *
 * Kind 24201 is ephemeral, so the relay never replays it — `since` only bounds
 * what a reconnect can surface. Kept short because a spawner re-sends its
 * request when the previous round times out; a stale frame the owner already
 * answered would just prompt them twice for nothing.
 */
const ATTESTATION_LOOKBACK_SECS = 120;

/** One AI provider a spawner can run agents against, from kind:10180 `ai`. */
export type SpawnerAiProvider = {
  id: string;
  models: string[];
};

/** A spawner advertising itself, from kind:10180. */
export type SpawnerAnnouncement = {
  /** Announcing pubkey — the verified event author, not a content field. */
  pubkey: string;
  name: string;
  description?: string;
  agentImage?: string;
  /** ACP agent binary the host runs. Display-only; the host decides, not us. */
  runtime?: string;
  maxAgents: number;
  agentsRunning: number;
  maxCpuMillis?: number;
  maxMemoryMib?: number;
  /** AI providers/models this spawner can run agents against, when advertised. */
  ai?: SpawnerAiProvider[];
};

/**
 * Subscribe to spawner announcements in this community.
 *
 * Unfiltered by author: discovery is the point. Everything in the content is
 * self-reported and must be treated as a hint — a spawner appearing here is not
 * trusted, and running an agent on it still requires the owner to sign an
 * attestation.
 */
export function subscribeToSpawnerAnnouncements(
  onEvent: (event: RelayEvent) => void,
) {
  return relayClient.subscribeLive(
    { kinds: [KIND_SPAWNER_ANNOUNCEMENT], limit: 100 },
    onEvent,
  );
}

/** Parse a kind:10180 event, returning null when unusable. */
export function parseSpawnerAnnouncement(
  event: RelayEvent,
): SpawnerAnnouncement | null {
  if (!event.content.trim()) return null;
  try {
    const raw = JSON.parse(event.content) as Record<string, unknown>;
    const name = typeof raw.name === "string" ? raw.name.trim() : "";
    // A nameless spawner would render as a blank row, which is worse than not
    // listing it at all.
    if (!name) return null;
    return {
      // Identity comes from the signed envelope, never from content.
      pubkey: event.pubkey,
      name,
      description: asOptionalString(raw.description),
      agentImage: asOptionalString(raw.agent_image),
      runtime: asOptionalString(raw.runtime),
      maxAgents: typeof raw.max_agents === "number" ? raw.max_agents : 0,
      agentsRunning:
        typeof raw.agents_running === "number" ? raw.agents_running : 0,
      maxCpuMillis:
        typeof raw.max_cpu_millis === "number" ? raw.max_cpu_millis : undefined,
      maxMemoryMib:
        typeof raw.max_memory_mib === "number" ? raw.max_memory_mib : undefined,
      ai: asOptionalAiCatalog(raw.ai),
    };
  } catch {
    return null;
  }
}

/** The reconciliation phase a spawner reports, mirroring `SpawnPhase` in Rust. */
export type SpawnPhase =
  | "pending_attestation"
  | "starting"
  | "running"
  | "failed"
  | "stopped";

/** Content body of a kind:30179 status event. */
export type SpawnerAgentStatus = {
  phase: SpawnPhase;
  agentPubkey?: string;
  specHash?: string;
  error?: string;
  restartCount: number;
  /** Hash of the prompt material last acknowledged by this agent, when reported. */
  promptHash?: string;
  /** True when the spawner holds this agent stopped awaiting an owner credential. */
  needsCredential: boolean;
};

/** Inbound author gate for a spawned agent, mirroring `RespondTo` in Rust. */
export type SpawnerRespondTo = "anyone" | "owner-only" | "allowlist";

/** Content body of a kind:30178 spec event. */
export type SpawnerAgentSpec = {
  name: string;
  /**
   * Existing agent to relocate, instead of minting a fresh key.
   *
   * The **public** key only — never a secret. Setting it tells the spawner
   * "this is an agent that already exists"; the secret is asked for separately
   * over the encrypted kind:24201 handshake, which is the only channel that can
   * carry it. A kind:30178 spec is world-readable, so a secret here would be
   * published to the whole community.
   *
   * Identity has to be preserved because the pubkey *is* the agent: its channel
   * membership, its kind:0 profile, the `d` tag of its kind:30177 record, its
   * DMs, and NIP-AE memory whose `d` tags derive from
   * `conversation_key(agent_seckey, owner_pubkey)`. A new key would leave that
   * memory permanently undecryptable.
   */
  agentPubkey?: string;
  personaId?: string;
  systemPrompt?: string;
  model?: string;
  provider?: string;
  parallelism: number;
  respondTo: SpawnerRespondTo;
  respondToAllowlist?: string[];
  resources?: { cpuMillis?: number; memoryMib?: number };
  enabled: boolean;
};

/**
 * Subscribe to attestation frames addressed to this owner.
 *
 * The relay routes kind:24201 on the global `#p`-kind index and enforces a
 * single `p` tag at ingest, so this filter receives one owner's handshakes and
 * nobody else's. The payload is still NIP-44 ciphertext — decryption happens in
 * Rust, where the owner's secret key lives.
 */
export function subscribeToSpawnerAttestations(
  ownerPubkey: string,
  onEvent: (event: RelayEvent) => void,
) {
  return relayClient.subscribeLive(
    {
      kinds: [KIND_SPAWNER_ATTESTATION],
      "#p": [ownerPubkey],
      limit: 100,
      since: Math.floor(Date.now() / 1_000) - ATTESTATION_LOOKBACK_SECS,
    },
    onEvent,
  );
}

/**
 * Subscribe to status events for this owner's server-hosted agents.
 *
 * Filtered by `#p` rather than by author: an owner may use more than one
 * spawner, and each tags the owner on the status events it publishes. Because
 * status is NIP-33 addressed by `(pubkey, kind, d_tag)`, an impostor's event
 * lands at their own address and cannot overwrite a real one — but it can still
 * reach this subscription, so callers must check `event.pubkey` against a
 * spawner they trust before believing it.
 */
export function subscribeToSpawnerStatus(
  ownerPubkey: string,
  onEvent: (event: RelayEvent) => void,
) {
  return relayClient.subscribeLive(
    { kinds: [KIND_SPAWNER_AGENT_STATUS], "#p": [ownerPubkey], limit: 200 },
    onEvent,
  );
}

/**
 * Publish (or replace) an agent spec addressed to `spawnerPubkey`.
 *
 * The `d` tag is the caller-chosen slug and is the stable handle for this agent
 * across edits — it cannot be the agent pubkey, which does not exist until the
 * spawner mints one.
 */
export async function publishSpawnerAgentSpec(input: {
  slug: string;
  spawnerPubkey: string;
  spec: SpawnerAgentSpec;
}): Promise<void> {
  await relayClient.preconnect();
  const event = await signRelayEvent({
    kind: KIND_SPAWNER_AGENT_SPEC,
    content: JSON.stringify(toWireSpec(input.spec)),
    tags: [
      ["d", input.slug],
      ["spawner", input.spawnerPubkey],
      ["p", input.spawnerPubkey],
    ],
  });
  await relayClient.publishEvent(
    event,
    "Timed out publishing the server agent.",
    "Failed to publish the server agent.",
  );
}

/**
 * Tear a server agent down permanently.
 *
 * Publishes an empty replacement rather than a kind:5 deletion: kind 30178 is
 * parameterized-replaceable, so a deletion leaves nothing for the spawner to
 * fan out and it would never learn the agent should stop. An emptied
 * replacement is the tombstone the spawner watches for, and it removes the
 * container, the volume, and the agent's key.
 *
 * To stop an agent while keeping its identity, publish the spec with
 * `enabled: false` instead.
 */
export async function deleteSpawnerAgentSpec(input: {
  slug: string;
  spawnerPubkey: string;
}): Promise<void> {
  await relayClient.preconnect();
  const event = await signRelayEvent({
    kind: KIND_SPAWNER_AGENT_SPEC,
    content: "",
    tags: [
      ["d", input.slug],
      ["spawner", input.spawnerPubkey],
      ["p", input.spawnerPubkey],
    ],
  });
  await relayClient.publishEvent(
    event,
    "Timed out removing the server agent.",
    "Failed to remove the server agent.",
  );
}

/**
 * Publish an attestation answer over the live WebSocket.
 *
 * Ephemeral kinds must not go through `POST /events`: that path runs the ingest
 * pipeline, whose per-kind scope allowlist covers stored kinds only and rejects
 * kind 24201 with `restricted: unknown event kind`. The relay's ephemeral
 * routing — and the `#p` gate that delivers the frame to exactly one
 * counterparty — lives in the WebSocket handler. Kind 24200 observer control
 * frames take the same route for the same reason.
 */
export async function respondToSpawnerAttestation(
  event: RelayEvent,
): Promise<void> {
  await relayClient.preconnect();
  await relayClient.publishEvent(
    event,
    "Timed out authorizing the server agent.",
    "Failed to authorize the server agent.",
  );
}

/**
 * Build and publish a prompt update for a server-hosted agent, over the same
 * WebSocket path as `respondToSpawnerAttestation`.
 *
 * The event is built by Rust (see `buildSpawnerPromptUpdate` in
 * `tauriSpawner.ts`), which is where the encrypted prompt material is
 * assembled and where the returned `promptHash` is computed — the renderer
 * never sees the plaintext prompt material re-derived independently.
 *
 * Returns the prompt hash so the caller can correlate it with the
 * `prompt_hash` the spawner later reports on kind:30179 status.
 */
export async function sendSpawnerPromptUpdate(input: {
  spawnerPubkey: string;
  specSlug: string;
  agentPubkey: string;
  prompt: SpawnerPromptMaterial;
}): Promise<string> {
  const { event, promptHash } = await buildSpawnerPromptUpdate(input);
  await relayClient.preconnect();
  await relayClient.publishEvent(
    event,
    "Timed out sending the prompt update.",
    "Failed to send the prompt update.",
  );
  return promptHash;
}

/**
 * Build and publish an owner credential update over the WebSocket — same
 * ephemeral-kind routing rationale as `sendSpawnerPromptUpdate`. Deliberately
 * no persistent queue: a queued plaintext credential on disk is exactly what
 * this feature exists to avoid. Confirmation arrives as an encrypted ack; see
 * `waitForSpawnerCredentialAck`.
 */
export async function sendSpawnerCredentialUpdate(input: {
  spawnerPubkey: string;
  credential: string;
}): Promise<void> {
  const { event } = await buildSpawnerCredentialUpdate(input);
  await relayClient.preconnect();
  await relayClient.publishEvent(
    event,
    "Timed out sending the credential.",
    "Failed to send the credential.",
  );
}

/** Parse a kind:30179 content body, returning null when it is unusable. */
export function parseSpawnerStatus(content: string): SpawnerAgentStatus | null {
  if (!content.trim()) return null;
  try {
    const raw = JSON.parse(content) as Record<string, unknown>;
    const phase = raw.phase;
    if (!isSpawnPhase(phase)) return null;
    return {
      phase,
      agentPubkey: asOptionalString(raw.agent_pubkey),
      specHash: asOptionalString(raw.spec_hash),
      error: asOptionalString(raw.error),
      restartCount:
        typeof raw.restart_count === "number" ? raw.restart_count : 0,
      promptHash: asOptionalString(raw.prompt_hash),
      needsCredential: raw.needs_credential === true,
    };
  } catch {
    return null;
  }
}

/** Read the `d` tag (spec slug) from a spawner event. */
export function specSlugFromEvent(event: RelayEvent): string | null {
  const tag = event.tags.find((t) => t[0] === "d" && t.length >= 2);
  return tag?.[1] ?? null;
}

/**
 * Convert the camelCase UI shape to the snake_case wire shape serde expects.
 *
 * Optional fields are omitted rather than sent as null: the Rust projection
 * uses `skip_serializing_if`, and an explicit null would fail to deserialize
 * into `Option<String>` on some fields while silently widening the wire
 * contract on others.
 */
function toWireSpec(spec: SpawnerAgentSpec): Record<string, unknown> {
  const wire: Record<string, unknown> = {
    name: spec.name,
    parallelism: spec.parallelism,
    respond_to: spec.respondTo,
    enabled: spec.enabled,
  };
  if (spec.agentPubkey) wire.agent_pubkey = spec.agentPubkey;
  if (spec.personaId) wire.persona_id = spec.personaId;
  if (spec.systemPrompt) wire.system_prompt = spec.systemPrompt;
  if (spec.model) wire.model = spec.model;
  if (spec.provider) wire.provider = spec.provider;
  if (spec.respondToAllowlist?.length) {
    wire.respond_to_allowlist = spec.respondToAllowlist;
  }
  if (spec.resources) {
    const resources: Record<string, unknown> = {};
    if (spec.resources.cpuMillis)
      resources.cpu_millis = spec.resources.cpuMillis;
    if (spec.resources.memoryMib)
      resources.memory_mib = spec.resources.memoryMib;
    if (Object.keys(resources).length) wire.resources = resources;
  }
  return wire;
}

function isSpawnPhase(value: unknown): value is SpawnPhase {
  return (
    value === "pending_attestation" ||
    value === "starting" ||
    value === "running" ||
    value === "failed" ||
    value === "stopped"
  );
}

function asOptionalString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

/**
 * Parse the `ai` catalog field, returning `undefined` for anything malformed.
 *
 * A spawner advertising a broken catalog is treated the same as one
 * advertising none at all — the picker just shows no models, rather than
 * guessing at a partial or corrupted list.
 */
function asOptionalAiCatalog(value: unknown): SpawnerAiProvider[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const providers: SpawnerAiProvider[] = [];
  for (const entry of value) {
    if (typeof entry !== "object" || entry === null) return undefined;
    const { id, models } = entry as Record<string, unknown>;
    if (typeof id !== "string" || !id) return undefined;
    if (!Array.isArray(models) || !models.every((m) => typeof m === "string")) {
      return undefined;
    }
    providers.push({ id, models });
  }
  return providers;
}
