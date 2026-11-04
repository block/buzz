// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { useUiLanguage } from "@/shared/i18n/uiTranslation";
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
  useUiLanguage();

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
