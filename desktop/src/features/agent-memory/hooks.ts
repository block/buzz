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

// TODO: temporary mock for Memories UI work — remove before merge.
const MOCK_AGENT_MEMORY: AgentMemoryListing = {
  core: {
    slug: "core",
    body: `I am a mock agent used to flesh out the Memories panel.

I prefer concise updates, explicit next steps, and visual polish before edge-case handling.

See [[mem/preferences/ui-density]] and [[mem/projects/sprout-memory-viewer]] for details.
Also referencing a deleted memory: [[mem/archive/deleted-plan]].`,
    eventId: "mock-core",
    createdAt: 1_700_000_000,
    outgoingRefs: [
      "mem/preferences/ui-density",
      "mem/projects/sprout-memory-viewer",
      "mem/archive/deleted-plan",
    ],
  },
  memories: [
    {
      slug: "mem/preferences/ui-density",
      body: "Prefer compact lists with generous body text when expanded.\n\nNested ref: [[mem/working-style/review-loop]]",
      eventId: "mock-ui-density",
      createdAt: 1_700_000_100,
      outgoingRefs: ["mem/working-style/review-loop"],
    },
    {
      slug: "mem/working-style/review-loop",
      body: "Ship small slices, screenshot the happy path, then iterate on empty/error states.",
      eventId: "mock-review-loop",
      createdAt: 1_700_000_200,
      outgoingRefs: [],
    },
    {
      slug: "mem/projects/sprout-memory-viewer",
      body: "Building the IXI-7 read-only memory viewer in the profile panel.\n\nChild memory: [[mem/projects/sprout-memory-viewer/notes]]",
      eventId: "mock-project",
      createdAt: 1_700_000_300,
      outgoingRefs: ["mem/projects/sprout-memory-viewer/notes"],
    },
    {
      slug: "mem/projects/sprout-memory-viewer/notes",
      body: "Tree should auto-expand core. Everything else collapsed with a one-line preview.",
      eventId: "mock-project-notes",
      createdAt: 1_700_000_400,
      outgoingRefs: [],
    },
    {
      slug: "mem/people/alice",
      body: "Alice prefers async updates in #design.",
      eventId: "mock-alice",
      createdAt: 1_700_000_500,
      outgoingRefs: [],
    },
    {
      slug: "mem/people/bob",
      body: "Bob reviews PRs quickly but wants screenshots.",
      eventId: "mock-bob",
      createdAt: 1_700_000_600,
      outgoingRefs: ["mem/people/alice"],
    },
    {
      slug: "mem/scratch/todo",
      body: "",
      eventId: "mock-empty",
      createdAt: 1_700_000_700,
      outgoingRefs: [],
    },
    {
      slug: "mem/orphan/unreferenced",
      body: "This memory is not reachable from core — should appear under Unreferenced.",
      eventId: "mock-orphan",
      createdAt: 1_700_000_800,
      outgoingRefs: ["mem/also-missing"],
    },
  ],
  truncated: true,
  fetchedAt: Math.floor(Date.now() / 1000),
};

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
    queryFn: () =>
      import.meta.env.DEV
        ? Promise.resolve(MOCK_AGENT_MEMORY)
        : getAgentMemory(agentPubkey as string),
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
