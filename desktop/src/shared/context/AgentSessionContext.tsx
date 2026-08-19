import * as React from "react";

type AgentSessionContextValue = {
  onOpenAgentSession:
    | ((pubkey: string, channelId?: string | null) => void)
    | null;
};

const AgentSessionContext = React.createContext<AgentSessionContextValue>({
  onOpenAgentSession: null,
});

export function AgentSessionProvider({
  children,
  onOpenAgentSession,
}: {
  children: React.ReactNode;
  onOpenAgentSession: (pubkey: string, channelId?: string | null) => void;
}) {
  const value = React.useMemo(
    () => ({ onOpenAgentSession }),
    [onOpenAgentSession],
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
