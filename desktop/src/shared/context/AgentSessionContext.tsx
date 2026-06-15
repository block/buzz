import * as React from "react";

type AgentSessionContextValue = {
  onOpenAgentSession: ((pubkey: string) => void) | null;
  /**
   * Opens the agent's activity log in a floating conversation window, separate
   * from the right-side thread panel. Null when no channel context is available
   * to scope the window to.
   */
  onOpenAgentWindow: ((pubkey: string) => void) | null;
};

const AgentSessionContext = React.createContext<AgentSessionContextValue>({
  onOpenAgentSession: null,
  onOpenAgentWindow: null,
});

export function AgentSessionProvider({
  children,
  onOpenAgentSession,
  onOpenAgentWindow = null,
}: {
  children: React.ReactNode;
  onOpenAgentSession: (pubkey: string) => void;
  onOpenAgentWindow?: ((pubkey: string) => void) | null;
}) {
  const value = React.useMemo(
    () => ({ onOpenAgentSession, onOpenAgentWindow }),
    [onOpenAgentSession, onOpenAgentWindow],
  );

  return (
    <AgentSessionContext.Provider value={value}>
      {children}
    </AgentSessionContext.Provider>
  );
}

export function useAgentSession() {
  return React.useContext(AgentSessionContext);
}
