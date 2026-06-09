import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useOpenDmMutation } from "@/features/channels/hooks";
import { UserProfilePanel } from "@/features/profile/ui/UserProfilePanel";
import { PulseView } from "@/features/pulse/ui/PulseView";
import { useIdentityQuery } from "@/shared/api/hooks";
import { ProfilePanelProvider } from "@/shared/context/ProfilePanelContext";
import { useThreadPanelWidth } from "@/shared/hooks/useThreadPanelWidth";

export function PulseScreen() {
  const identityQuery = useIdentityQuery();
  const [profilePanelPubkey, setProfilePanelPubkey] = React.useState<
    string | null
  >(null);
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
    <ProfilePanelProvider onOpenProfilePanel={setProfilePanelPubkey}>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <div className="flex min-h-0 min-w-0 flex-1 flex-row overflow-hidden">
          <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
            <PulseView currentPubkey={identityQuery.data?.pubkey} />
          </div>
          {profilePanelPubkey ? (
            <UserProfilePanel
              canResetWidth={threadPanelWidth.canReset}
              currentPubkey={identityQuery.data?.pubkey}
              onClose={() => setProfilePanelPubkey(null)}
              onOpenDm={handleOpenDm}
              onResetWidth={threadPanelWidth.onResetWidth}
              onResizeStart={threadPanelWidth.onResizeStart}
              pubkey={profilePanelPubkey}
              widthPx={threadPanelWidth.widthPx}
            />
          ) : null}
        </div>
      </div>
    </ProfilePanelProvider>
  );
}
