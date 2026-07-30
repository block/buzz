import { invokeTauri } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";

/// The owner's trust decision for a spawner pubkey, as the Rust side spells it.
export type SpawnerTrust = "trusted" | "untrusted";

/**
 * Prompt material sent to a spawner over the encrypted handshake.
 *
 * Travels here rather than on the kind:30178 spec because specs are
 * world-readable: inlining a system prompt there — or marking the persona
 * `["shared","true"]` — would publish it to the whole community. Snake_case to
 * match the Rust `PromptMaterial` it deserializes into.
 */
export type SpawnerPromptMaterial = {
  system_prompt?: string;
  team_instructions?: string;
  model?: string;
  provider?: string;
};

/// A decrypted kind:24201 attestation request, ready to show the user.
export type SpawnerAttestationRequest = {
  spawnerPubkey: string;
  specSlug: string;
  agentPubkey: string;
  conditions: string;
  nonce: string;
};

/**
 * Decrypt an inbound attestation frame so the UI can show what is being asked.
 *
 * Returns `null` for frames that are not requests — a response or rejection is
 * the owner's own outbound traffic echoed back off the ephemeral stream, and
 * must not raise a second prompt.
 */
export async function decodeSpawnerAttestation(
  spawnerPubkey: string,
  encryptedContent: string,
): Promise<SpawnerAttestationRequest | null> {
  const result = await invokeTauri<SpawnerAttestationRequest | null>(
    "decode_spawner_attestation",
    { spawnerPubkey, encryptedContent },
  );
  return result ?? null;
}

/**
 * Build the signed answer to an attestation request.
 *
 * The ciphertext is passed back rather than the decoded fields: Rust
 * re-decrypts and signs over the agent pubkey *it* reads from the frame, so a
 * renderer bug cannot substitute a different key between the prompt the user
 * saw and the tag that gets signed.
 *
 * Returns the signed event for the caller to publish over the WebSocket — see
 * `respondToSpawnerAttestation` in `spawnerRelay.ts` for why HTTP will not do.
 */
export async function buildSpawnerAttestationResponse(input: {
  spawnerPubkey: string;
  encryptedContent: string;
  trust: SpawnerTrust;
  rejectReason?: string;
  prompt?: SpawnerPromptMaterial;
}): Promise<SpawnerAttestationResponse> {
  const raw = await invokeTauri<RawSpawnerAttestationResponse>(
    "respond_to_spawner_attestation",
    {
      spawnerPubkey: input.spawnerPubkey,
      encryptedContent: input.encryptedContent,
      trust: input.trust,
      rejectReason: input.rejectReason ?? null,
      prompt: input.prompt ?? null,
    },
  );
  return {
    event: JSON.parse(raw.eventJson) as RelayEvent,
    relocatedAgentPubkey: raw.relocatedAgentPubkey ?? null,
  };
}

/** Wire shape of `respond_to_spawner_attestation`. */
type RawSpawnerAttestationResponse = {
  eventJson: string;
  relocatedAgentPubkey?: string | null;
};

/**
 * Build a signed prompt-update event for a server-hosted agent, plus the
 * hash of the prompt material it carries.
 *
 * Rust builds and signs the event so the returned `promptHash` is always
 * computed over the same bytes that got published — the renderer never
 * recomputes it independently. The caller is responsible for publishing the
 * returned event over the WebSocket; see `sendSpawnerPromptUpdate` in
 * `spawnerRelay.ts`.
 */
export async function buildSpawnerPromptUpdate(input: {
  spawnerPubkey: string;
  specSlug: string;
  agentPubkey: string;
  prompt: SpawnerPromptMaterial;
}): Promise<{ event: RelayEvent; promptHash: string }> {
  const raw = await invokeTauri<RawSpawnerPromptUpdate>(
    "send_spawner_prompt_update",
    {
      spawnerPubkey: input.spawnerPubkey,
      specSlug: input.specSlug,
      agentPubkey: input.agentPubkey,
      prompt: input.prompt,
    },
  );
  return {
    event: JSON.parse(raw.eventJson) as RelayEvent,
    promptHash: raw.promptHash,
  };
}

/** Wire shape of `send_spawner_prompt_update`. */
type RawSpawnerPromptUpdate = {
  eventJson: string;
  promptHash: string;
};

/** A decoded credential ack from a spawner. */
export type SpawnerCredentialAck = {
  accepted: boolean;
  message?: string | null;
};

/**
 * Build a signed credential update for a spawner. The token goes straight to
 * Rust, which encrypts it into the returned event — it is never persisted on
 * this device. An empty `credential` clears the spawner-side token.
 */
export async function buildSpawnerCredentialUpdate(input: {
  spawnerPubkey: string;
  credential: string;
}): Promise<{ event: RelayEvent }> {
  const raw = await invokeTauri<{ eventJson: string }>(
    "send_spawner_credential_update",
    { spawnerPubkey: input.spawnerPubkey, credential: input.credential },
  );
  return { event: JSON.parse(raw.eventJson) as RelayEvent };
}

/** Decode an inbound 24201 frame as a credential ack; null for other frames. */
export async function decodeSpawnerCredentialAck(
  spawnerPubkey: string,
  encryptedContent: string,
): Promise<SpawnerCredentialAck | null> {
  const result = await invokeTauri<SpawnerCredentialAck | null>(
    "decode_spawner_credential_ack",
    { spawnerPubkey, encryptedContent },
  );
  return result ?? null;
}

/**
 * The signed answer, plus whether it handed over an existing agent's key.
 *
 * `relocatedAgentPubkey` is set by Rust — not by the renderer — when the
 * requested agent pubkey turned out to be a locally managed agent and its nsec
 * was included in the encrypted response. That makes it the authoritative
 * signal that this agent now lives on the server, and the caller must stop the
 * local process: two processes on one key both answer every mention.
 */
export type SpawnerAttestationResponse = {
  event: RelayEvent;
  relocatedAgentPubkey: string | null;
};
