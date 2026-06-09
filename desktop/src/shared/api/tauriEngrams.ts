import { invokeTauri } from "@/shared/api/tauri";

// ── NIP-AE agent memory (engram) reads ──────────────────────────────────────

/**
 * One memory entry. `slug` is the canonical engram slug — `core` for the
 * agent's identity profile, `mem/...` for everything else. `body` is the
 * decrypted UTF-8 payload. `outgoingRefs` is the list of `[[slug]]`
 * references parsed from the body; the UI BFSes from `core.outgoingRefs`
 * to compute reachable vs orphan sets.
 */
export type EngramEntry = {
  slug: string;
  body: string;
  eventId: string;
  /** Unix seconds. */
  createdAt: number;
  outgoingRefs: string[];
};

/**
 * Response shape for `get_agent_memory`. `core` is split from `memories`
 * because the UI roots its reachability tree there. `truncated` flags a
 * relay cap hit (>= 5000 events for this (agent, owner) pair). `fetchedAt`
 * (unix seconds) is for "last loaded" copy on the refetch affordance.
 */
export type AgentMemoryListing = {
  core: EngramEntry | null;
  memories: EngramEntry[];
  truncated: boolean;
  fetchedAt: number;
};

/**
 * Owner-gated single-payload engram listing. The Rust side enforces that
 * the requested agent appears in this desktop's `managed_agents` store
 * before deriving the conversation key or attempting decrypt — non-owners
 * receive an `Err` (and the UI hides the section anyway).
 *
 * Throws on:
 * - non-hex agent pubkey
 * - viewer is not the agent's owner
 * - relay query failure
 *
 * Returns `{ core: null, memories: [] }` when the agent has no engrams —
 * that's the legitimate empty state, distinct from a thrown error.
 */
export async function getAgentMemory(
  agentPubkey: string,
): Promise<AgentMemoryListing> {
  return invokeTauri<AgentMemoryListing>("get_agent_memory", {
    agentPubkey,
  });
}
