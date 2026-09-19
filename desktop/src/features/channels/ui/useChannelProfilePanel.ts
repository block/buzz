import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useOpenDmMutation } from "@/features/channels/hooks";
import type { ProfilePanelOpenOptions } from "@/shared/context/ProfilePanelContext";

type UseChannelProfilePanelOptions = {
  closeAgentSession: () => void;
  openProfilePanel: (pubkey: string, options?: ProfilePanelOpenOptions) => void;
  requireThreadEditResolution: () => boolean;
  setChannelManagementOpen: (open: boolean) => void;
  setExpandedThreadReplyIds: (value: Set<string>) => void;
  setOpenThreadHeadId: (value: string | null) => void;
  setProfilePanelPubkey: (value: string | null) => void;
  setThreadReplyTargetId: (value: string | null) => void;
  setThreadScrollTargetId: (value: string | null) => void;
};

export function useChannelProfilePanel({
  closeAgentSession,
  openProfilePanel,
  requireThreadEditResolution,
  setChannelManagementOpen,
  setExpandedThreadReplyIds,
  setOpenThreadHeadId,
  setProfilePanelPubkey,
  setThreadReplyTargetId,
  setThreadScrollTargetId,
}: UseChannelProfilePanelOptions) {
  const { goChannel } = useAppNavigation();
  const openDmMutation = useOpenDmMutation();

  const handleOpenProfilePanel = React.useCallback(
    (pubkey: string, options?: ProfilePanelOpenOptions) => {
      if (!requireThreadEditResolution()) return;
      setOpenThreadHeadId(null);
      setExpandedThreadReplyIds(new Set());
      setThreadScrollTargetId(null);
      setThreadReplyTargetId(null);
      closeAgentSession();
      setChannelManagementOpen(false);
      openProfilePanel(pubkey, options);
    },
    [
      closeAgentSession,
      openProfilePanel,
      requireThreadEditResolution,
      setChannelManagementOpen,
      setExpandedThreadReplyIds,
      setOpenThreadHeadId,
      setThreadReplyTargetId,
      setThreadScrollTargetId,
    ],
  );

  const handleCloseProfilePanel = React.useCallback(() => {
    setProfilePanelPubkey(null);
  }, [setProfilePanelPubkey]);

  const openDmMutateAsync = openDmMutation.mutateAsync;
  const handleOpenDm = React.useCallback(
    async (pubkeys: string[]) => {
      const dm = await openDmMutateAsync({ pubkeys });
      await goChannel(dm.id);
    },
    [goChannel, openDmMutateAsync],
  );

  return {
    handleOpenProfilePanel,
    handleCloseProfilePanel,
    handleOpenDm,
  };
}
