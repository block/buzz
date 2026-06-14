import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useOpenDmMutation } from "@/features/channels/hooks";
import {
  type ProfilePanelView,
  UserProfilePanel,
} from "@/features/profile/ui/UserProfilePanel";
import { PulseView } from "@/features/pulse/ui/PulseView";
import { useIdentityQuery } from "@/shared/api/hooks";
import { ProfilePanelProvider } from "@/shared/context/ProfilePanelContext";
import { useHistorySearchState } from "@/shared/hooks/useHistorySearchState";
import { useThreadPanelWidth } from "@/shared/hooks/useThreadPanelWidth";

const PULSE_PANEL_SEARCH_KEYS = ["profile", "profileView"] as const;

export function PulseScreen() {
  const identityQuery = useIdentityQuery();
  const { applyPatch, values } = useHistorySearchState(PULSE_PANEL_SEARCH_KEYS);
  const profilePanelPubkey = values.profile;
  const profilePanelView: ProfilePanelView =
    values.profileView === "memories" || values.profileView === "channels"
      ? values.profileView
      : "summary";
  const handleOpenProfilePanel = React.useCallback(
    (pubkey: string) => applyPatch({ profile: pubkey, profileView: null }),
    [applyPatch],
  );
  const handleCloseProfilePanel = React.useCallback(
    () => applyPatch({ profile: null, profileView: null }),
    [applyPatch],
  );
  const handleProfilePanelViewChange = React.useCallback(
    (view: ProfilePanelView, options?: { replace?: boolean }) =>
      applyPatch({ profileView: view === "summary" ? null : view }, options),
    [applyPatch],
  );
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
    <ProfilePanelProvider onOpenProfilePanel={handleOpenProfilePanel}>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <div className="flex min-h-0 min-w-0 flex-1 flex-row overflow-hidden">
          <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
            <PulseView currentPubkey={identityQuery.data?.pubkey} />
          </div>
          {profilePanelPubkey ? (
            <UserProfilePanel
              canResetWidth={threadPanelWidth.canReset}
              currentPubkey={identityQuery.data?.pubkey}
              onClose={handleCloseProfilePanel}
              onOpenDm={handleOpenDm}
              onResetWidth={threadPanelWidth.onResetWidth}
              onResizeStart={threadPanelWidth.onResizeStart}
              onViewChange={handleProfilePanelViewChange}
              pubkey={profilePanelPubkey}
              view={profilePanelView}
              widthPx={threadPanelWidth.widthPx}
            />
          ) : null}
        </div>
      </div>
    </ProfilePanelProvider>
  );
}
