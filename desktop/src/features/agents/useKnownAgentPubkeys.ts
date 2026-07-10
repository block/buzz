import * as React from "react";

import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { mergeKnownAgentPubkeys } from "@/features/agents/knownAgentPubkeys";
import { useHomeFeedQuery } from "@/features/home/hooks";
import { useStableSet } from "@/shared/hooks/useStableReference";

/**
 * The workspace-scoped "known agent pubkeys" baseline: locally managed agents
 * ∪ relay-registered agents ∪ home-feed agent activity, normalised via
 * `normalizePubkey`.
 *
 * Every surface that decides whether a pubkey belongs to an agent — the
 * config-nudge trust gate, bot avatars/popovers, agent mention pills — must
 * share this baseline. Surfaces previously derived their own sets from
 * different source subsets, so the same event could pass the trust gate on
 * one screen and fail it on another.
 *
 * Surface-local signals stay additive on top: merge channel-member roles or
 * a profile lookup's `isAgent` flags at the call site (or check
 * `profiles[normalizePubkey(pk)]?.isAgent` per pubkey). They can only widen
 * the baseline, never diverge from it.
 *
 * Backed by React Query, so the set follows workspace switches without a
 * `resetWorkspaceState()` entry. All three source queries are already
 * mounted app-wide (AppShell polls the home feed; agent queries back the
 * sidebar and channel surfaces), so extra consumers add observers, not
 * fetches. The returned set is content-stable across source-data churn that
 * doesn't change membership (e.g. a managed agent's status flip), so it is
 * safe as a memo/comparator dependency in render-hot consumers.
 */
export function useKnownAgentPubkeys(): ReadonlySet<string> {
  const managedAgents = useManagedAgentsQuery().data;
  const relayAgents = useRelayAgentsQuery().data;
  const feedAgentActivity = useHomeFeedQuery().data?.feed.agentActivity;

  const merged = React.useMemo(
    () => mergeKnownAgentPubkeys(managedAgents, relayAgents, feedAgentActivity),
    [feedAgentActivity, managedAgents, relayAgents],
  );

  return useStableSet(merged);
}
