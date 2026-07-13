import * as React from "react";

import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { mergeKnownAgentPubkeys } from "@/features/agents/knownAgentPubkeys";
import { useStableSet } from "@/shared/hooks/useStableReference";

const EMPTY_KNOWN_AGENT_PUBKEYS: ReadonlySet<string> = new Set();

const KnownAgentPubkeysContext = React.createContext<ReadonlySet<string>>(
  EMPTY_KNOWN_AGENT_PUBKEYS,
);

/**
 * Owns the app's only React Query subscription to the known-agent source
 * queries and publishes the merged set over context.
 *
 * The subscription lives here — not in `useKnownAgentPubkeys` — on purpose.
 * Consumers include every mounted `MessageRow`; if each consumer held its own
 * query observers, every batch of row mounts (channel switch, thread panel
 * open, load-older) would find short-staleTime data stale and re-trigger
 * fetches via `refetchOnMount`, including the deliberately relaxed
 * whole-profile-set `listRelayAgents` relay query. And each source-query data
 * churn (managed agents poll at 5s while an agent runs) would re-render every
 * row before `useStableSet` could bail. With the single subscription here,
 * query churn re-renders only this provider — `children` is referentially
 * stable, so nothing cascades — and context consumers re-render only when the
 * published set's identity changes, which `useStableSet` restricts to actual
 * membership change.
 *
 * Mounted once per workspace inside `AppReady` (under the workspace-keyed
 * `WorkspaceQueryProvider` remount boundary), so the observers tear down and
 * re-create on workspace switch without a `resetWorkspaceState()` entry.
 */
export function KnownAgentPubkeysProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const managedAgents = useManagedAgentsQuery().data;
  const relayAgents = useRelayAgentsQuery().data;

  const merged = React.useMemo(
    () => mergeKnownAgentPubkeys(managedAgents, relayAgents),
    [managedAgents, relayAgents],
  );
  const stable = useStableSet(merged);

  return (
    <KnownAgentPubkeysContext.Provider value={stable}>
      {children}
    </KnownAgentPubkeysContext.Provider>
  );
}

/**
 * The workspace-scoped "known agent pubkeys" baseline: locally managed agents
 * ∪ relay-registered agents, normalised via `normalizePubkey`. Home-feed agent
 * activity is intentionally excluded: it is a display category, not an
 * authenticated agent-identity source.
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
 * Reads content-stable context published by `KnownAgentPubkeysProvider` —
 * consumers add no query observers and re-render only when membership
 * actually changes, so the set is safe as a memo/comparator dependency in
 * render-hot consumers. Outside the provider (unit tests, stray surfaces)
 * this degrades gracefully to the empty set; surfaces still fold in their
 * local `isAgent` profile flags.
 */
export function useKnownAgentPubkeys(): ReadonlySet<string> {
  return React.useContext(KnownAgentPubkeysContext);
}
