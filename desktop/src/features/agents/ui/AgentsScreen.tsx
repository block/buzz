import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useOpenDmMutation } from "@/features/channels/hooks";
import { UserProfilePanel } from "@/features/profile/ui/UserProfilePanel";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { AgentPersona } from "@/shared/api/types";
import { ProfilePanelProvider } from "@/shared/context/ProfilePanelContext";
import { useThreadPanelWidth } from "@/shared/hooks/useThreadPanelWidth";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const AgentsView = React.lazy(async () => {
  const module = await import("@/features/agents/ui/AgentsView");
  return { default: module.AgentsView };
});

type ProfilePanelTarget =
  | { kind: "pubkey"; pubkey: string }
  | { kind: "persona"; persona: AgentPersona };

export function AgentsScreen() {
  const identityQuery = useIdentityQuery();
  const [profilePanelTarget, setProfilePanelTarget] =
    React.useState<ProfilePanelTarget | null>(null);
  const threadPanelWidth = useThreadPanelWidth();
  const openDmMutation = useOpenDmMutation();
  const { goChannel } = useAppNavigation();

  const handleOpenDm = React.useCallback(
    async (pubkeys: string[]) => {
      const dm = await openDmMutation.mutateAsync({ pubkeys });
      await goChannel(dm.id);
    },
    [goChannel, openDmMutation],
  );

  return (
    <ProfilePanelProvider
      onOpenPersonaProfilePanel={(persona) =>
        setProfilePanelTarget({ kind: "persona", persona })
      }
      onOpenProfilePanel={(pubkey) =>
        setProfilePanelTarget({ kind: "pubkey", pubkey })
      }
    >
      <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <div className="flex min-h-0 min-w-0 flex-1 flex-row overflow-hidden">
          <React.Suspense fallback={<ViewLoadingFallback kind="agents" />}>
            <AgentsView />
          </React.Suspense>
          {profilePanelTarget ? (
            <UserProfilePanel
              canResetWidth={threadPanelWidth.canReset}
              currentPubkey={identityQuery.data?.pubkey}
              onClose={() => setProfilePanelTarget(null)}
              onOpenDm={handleOpenDm}
              onResetWidth={threadPanelWidth.onResetWidth}
              onResizeStart={threadPanelWidth.onResizeStart}
              persona={
                profilePanelTarget.kind === "persona"
                  ? profilePanelTarget.persona
                  : undefined
              }
              pubkey={
                profilePanelTarget.kind === "pubkey"
                  ? profilePanelTarget.pubkey
                  : undefined
              }
              widthPx={threadPanelWidth.widthPx}
            />
          ) : null}
        </div>
      </div>
    </ProfilePanelProvider>
  );
}
