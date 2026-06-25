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
 * Synchronous gate: does this desktop manage the agent (i.e. hold its
 * seckey locally)? This is the *local fast-path* owner signal — one of two
 * inputs to `viewerIsOwner`. The other is the declared NIP-OA owner signal
 * (`isCurrentUserOwner`, computed in the profile panel from the agent's
 * `kind:0`). Callers OR the two: `viewerIsOwner = isCurrentUserOwner || isOwner`.
 *
 * Returns `boolean | undefined`:
 *   - `undefined` is the *loading* state (managed-agent list still resolving).
 *     Callers should defer rendering, not show an error.
 *   - `true` / `false` once the list is known.
 *
 * Why not gate the Memory section on this alone? It answers "do I hold the
 * seckey?", which used to stand in for "am I the owner?". Those two diverge
 * exactly for a remote-owned agent — the owner runs it on another desktop,
 * holds no local seckey, but legitimately owns it. Engrams are NIP-44
 * encrypted to the *owner's* pubkey and decrypted with the owner's own key
 * (never the agent's seckey), so a declared owner can read their own memory
 * regardless of where the agent runs. The encryption is the real boundary;
 * this predicate is just the no-roundtrip half of the owner check. The
 * declared-owner half (`isCurrentUserOwner`) covers the remote case.
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
 * the section for non-owners (the caller passes `viewerIsOwner` down to
 * {@link MemorySection}), but this hook is robust to a misuse there.
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
