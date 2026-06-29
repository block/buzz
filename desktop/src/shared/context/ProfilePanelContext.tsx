import * as React from "react";

import type { AgentPersona } from "@/shared/api/types";

export type ProfilePanelOpenOptions = {
  tab?: "info" | "runtime" | "channels" | "memories";
};

type ProfilePanelContextValue = {
  openProfilePanel:
    | ((pubkey: string, options?: ProfilePanelOpenOptions) => void)
    | null;
  openPersonaProfilePanel: ((persona: AgentPersona) => void) | null;
};

const ProfilePanelContext = React.createContext<ProfilePanelContextValue>({
  openProfilePanel: null,
  openPersonaProfilePanel: null,
});

export function ProfilePanelProvider({
  children,
  onOpenProfilePanel,
  onOpenPersonaProfilePanel,
}: {
  children: React.ReactNode;
  onOpenProfilePanel: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  onOpenPersonaProfilePanel?: (persona: AgentPersona) => void;
}) {
  const value = React.useMemo(
    () => ({
      openProfilePanel: onOpenProfilePanel,
      openPersonaProfilePanel: onOpenPersonaProfilePanel ?? null,
    }),
    [onOpenPersonaProfilePanel, onOpenProfilePanel],
  );

  return (
    <ProfilePanelContext.Provider value={value}>
      {children}
    </ProfilePanelContext.Provider>
  );
}

export function useProfilePanel() {
  return React.useContext(ProfilePanelContext);
}
