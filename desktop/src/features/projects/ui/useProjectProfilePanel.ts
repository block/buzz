import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useOpenDmMutation } from "@/features/channels/hooks";
import type {
  ProfilePanelTab,
  ProfilePanelView,
} from "@/features/profile/ui/UserProfilePanel";
import {
  profilePanelTabFromSearch,
  profilePanelViewFromSearch,
} from "@/features/profile/ui/UserProfilePanelUtils";
import { useHistorySearchState } from "@/shared/hooks/useHistorySearchState";

const PROJECT_DETAIL_PANEL_SEARCH_KEYS = [
  "profile",
  "profileTab",
  "profileView",
] as const;

/** Owns URL-backed profile panel state for the project workspace. */
export function useProjectProfilePanel() {
  const { goChannel } = useAppNavigation();
  const { applyPatch, values } = useHistorySearchState(
    PROJECT_DETAIL_PANEL_SEARCH_KEYS,
  );
  const openDmMutation = useOpenDmMutation();

  return {
    profilePanelPubkey: values.profile,
    profilePanelTab: profilePanelTabFromSearch(values.profileTab),
    profilePanelView: profilePanelViewFromSearch(values.profileView),
    handleOpenProfilePanel: React.useCallback(
      (pubkey: string) =>
        applyPatch({ profile: pubkey, profileTab: null, profileView: null }),
      [applyPatch],
    ),
    handleCloseProfilePanel: React.useCallback(
      () => applyPatch({ profile: null, profileTab: null, profileView: null }),
      [applyPatch],
    ),
    handleProfilePanelViewChange: React.useCallback(
      (view: ProfilePanelView, options?: { replace?: boolean }) =>
        applyPatch({ profileView: view === "summary" ? null : view }, options),
      [applyPatch],
    ),
    handleProfilePanelTabChange: React.useCallback(
      (tab: ProfilePanelTab, options?: { replace?: boolean }) =>
        applyPatch({ profileTab: tab === "info" ? null : tab }, options),
      [applyPatch],
    ),
    handleOpenDm: React.useCallback(
      async (pubkeys: string[]) => {
        const dm = await openDmMutation.mutateAsync({ pubkeys });
        await goChannel(dm.id);
      },
      [goChannel, openDmMutation],
    ),
  };
}
