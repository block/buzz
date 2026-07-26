import type { SpawnerAgentSpec } from "@/shared/api/spawnerRelay";
import { personaDTag, slugFromName } from "./spawnerPreference";

/**
 * The fields of a local agent that a relocation needs.
 *
 * Structural rather than `ManagedAgent` so this stays a pure, testable function
 * of three strings instead of dragging the whole managed-agent shape in.
 */
export type RelocatableAgent = {
  pubkey: string;
  name: string;
  personaId?: string | null;
};

/** A relocation spec, ready to publish at `slug`. */
export type RelocationPlan = {
  slug: string;
  spec: SpawnerAgentSpec;
};

/**
 * Build the kind:30178 spec that moves an existing agent onto a spawner.
 *
 * What makes this a *relocation* rather than a second agent is `agentPubkey`:
 * the spawner reads it as "this agent already exists" and asks for its secret
 * over the encrypted handshake instead of minting a new key. Keeping the key is
 * the whole point — it carries the agent's channel membership, its profile, its
 * kind:30177 record (whose `d` tag is the pubkey itself), its DMs, and NIP-AE
 * memory that a new key could never decrypt.
 *
 * Only the public key goes on the spec. Specs are world-readable.
 *
 * Throws when the agent's name yields no usable slug, because the slug becomes
 * a container and volume name on the host and the Rust side rejects anything
 * outside `[a-z0-9_-]`.
 */
export function buildRelocationPlan(agent: RelocatableAgent): RelocationPlan {
  const slug = slugFromName(agent.name);
  if (!slug) {
    throw new Error(
      `"${agent.name}" has no usable server name. Rename it using letters or digits, then move it.`,
    );
  }
  return {
    slug,
    spec: {
      name: agent.name,
      agentPubkey: agent.pubkey,
      // A persona reference, never the prompt: the prompt travels over the
      // encrypted handshake. Without a persona the spawner still needs
      // *something* to validate against, so fall back to a placeholder the
      // handshake overrides.
      ...(agent.personaId
        ? { personaId: personaDTag(agent.personaId) }
        : { systemPrompt: "Server-hosted Buzz agent." }),
      parallelism: 1,
      respondTo: "anyone",
      enabled: true,
    },
  };
}

/**
 * Whether an attestation request is asking for an agent this device already
 * runs.
 *
 * Pubkeys are compared case-insensitively: hex from the relay and hex from the
 * local store are the same key regardless of casing, and a mismatch here would
 * silently downgrade the relocation warning to "a new key was created".
 */
export function isRelocationOfLocalAgent(
  agentPubkey: string | null | undefined,
  localAgentPubkeys: readonly string[],
): boolean {
  if (!agentPubkey) return false;
  const target = agentPubkey.toLowerCase();
  return localAgentPubkeys.some((pubkey) => pubkey.toLowerCase() === target);
}
