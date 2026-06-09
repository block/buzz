import * as React from "react";
import { useQuery } from "@tanstack/react-query";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  getAgentMemory,
  type AgentMemoryListing,
} from "@/shared/api/tauriEngrams";
import { buildMemoryGraph, type MemoryGraph } from "./lib/buildMemoryGraph";

export const agentMemoryQueryKey = (agentPubkey: string) =>
  ["agent-memory", agentPubkey.toLowerCase()] as const;

/**
 * Synchronous gate: does this desktop manage the agent? Used by the profile
 * panel to hide the Memory section entirely for non-owners.
 *
 * Returns `boolean | undefined`:
 *   - `undefined` is the *loading* state (managed-agent list still resolving).
 *     Callers should defer rendering, not show an error.
 *   - `true` / `false` once the list is known.
 *
 * Why `managed_agents` (not NIP-OA `kind:0` via `useOaOwnerQuery`)?
 * The archive button gates on `useOaOwnerQuery` because publishing a
 * `kind:9035` requires verifying NIP-OA cryptographically — the action is
 * *signing as the OA owner*. The memory viewer's question is different:
 * "do I have the seckey to decrypt this agent's engrams?" `managed_agents`
 * answers exactly that — it's the local source of truth for "agents whose
 * keys this desktop holds." NIP-OA on its own is weaker for this surface:
 * a malicious agent can forge an `auth` tag in their `kind:0` pointing at
 * any pubkey, but only the desktop that actually holds the seckey can
 * decrypt. Don't "fix" this back to `useOaOwnerQuery` — it would replace
 * a precise predicate with a weaker one and add a relay roundtrip.
 *
 * Lowercase compare because pubkeys can arrive from either side in mixed
 * case via Nostr libs; the underlying store stores them as-given.
 */
export function useIsManagedAgent(
  agentPubkey: string | null | undefined,
): boolean | undefined {
  const query = useManagedAgentsQuery();
  if (!agentPubkey) return false;
  if (!query.data) return undefined;
  const lower = agentPubkey.toLowerCase();
  return query.data.some((m) => m.pubkey.toLowerCase() === lower);
}

/**
 * Fetch + decrypt the engram listing for one agent. Owner-gated at the
 * Rust layer; if the viewer isn't the agent's owner the underlying call
 * returns an `Err` (we surface it as `query.isError`). The UI must hide
 * the section for non-owners — see {@link useIsManagedAgent} — but this
 * hook is robust to a misuse there.
 *
 * `staleTime: 30s`: engrams change rarely (each write is a deliberate
 * agent action). 30s keeps profile re-opens snappy without going so far
 * that the user sees stale data after their agent edits a memory in the
 * background. Refetch is one-tap via `query.refetch()`.
 *
 * `enabled` defaults to true; pass `false` from a non-owner caller (or
 * when no agent is selected) to skip the call entirely.
 */
export function useAgentMemoryQuery(
  agentPubkey: string | null | undefined,
  options?: { enabled?: boolean },
) {
  const enabled = (options?.enabled ?? true) && !!agentPubkey;
  return useQuery<AgentMemoryListing>({
    enabled,
    queryKey: agentMemoryQueryKey(agentPubkey ?? ""),
    queryFn: () => getAgentMemory(agentPubkey as string),
    staleTime: 30_000,
  });
}

/**
 * Convenience wrapper: feeds the listing through {@link buildMemoryGraph}
 * and memoizes the result. The graph is computed in JS (off the Rust
 * boundary) because it's a pure function of the payload; recomputing on
 * every render is cheap but not free for large agents (IXI-60 will
 * worker-ize this if the numbers warrant).
 */
export function useAgentMemoryGraph(
  agentPubkey: string | null | undefined,
  options?: { enabled?: boolean },
): {
  query: ReturnType<typeof useAgentMemoryQuery>;
  graph: MemoryGraph | null;
} {
  const query = useAgentMemoryQuery(agentPubkey, options);
  const graph = React.useMemo<MemoryGraph | null>(() => {
    if (!query.data) return null;
    return buildMemoryGraph(query.data);
  }, [query.data]);
  return { query, graph };
}
