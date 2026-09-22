import * as React from "react";

import type { AgentRunLocation } from "../lib/agentAccessWarning";

/**
 * Where the agent a dialog subtree is editing will run.
 *
 * Context rather than a prop on purpose: the only consumer is the respond-to
 * warning, buried several levels inside `AgentDefinitionDialog` (1000+ lines)
 * and `AgentInstanceEditDialog` (1200+ lines). Threading a prop through them
 * would grow two files that are already over the 1000-line ceiling enforced by
 * `desktop/scripts/check-file-sizes.mjs`, for a value neither of them uses.
 *
 * `AgentDialog` is the single entry point for all three dialog modes, so it is
 * the one place that provides this. Surfaces that render the respond-to field
 * outside that tree (e.g. `EditRespondToDialog`) pass the `runLocation` prop
 * directly instead.
 *
 * The default is `null` — unknown. Never provide a guessed value; the copy
 * falls back to the local wording, which is correct for any owner who has not
 * installed a `buzz-backend-*` provider.
 */
const AgentRunLocationContext = React.createContext<AgentRunLocation | null>(
  null,
);

export function AgentRunLocationProvider({
  children,
  runLocation,
}: {
  children: React.ReactNode;
  runLocation: AgentRunLocation | null;
}) {
  return (
    <AgentRunLocationContext.Provider value={runLocation}>
      {children}
    </AgentRunLocationContext.Provider>
  );
}

export function useAgentRunLocation(): AgentRunLocation | null {
  return React.useContext(AgentRunLocationContext);
}
