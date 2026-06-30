import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useOpenDmMutation } from "@/features/channels/hooks";

type UseChannelProfilePanelOptions = {
  closeAgentSession: () => void;
  setChannelManagementOpen: (open: boolean) => void;
  setExpandedThreadReplyIds: (value: Set<string>) => void;
  setOpenThreadHeadId: (value: string | null) => void;
  setProfilePanelPubkey: (value: string | null) => void;
  setThreadReplyTargetId: (value: string | null) => void;
  setThreadScrollTargetId: (value: string | null) => void;
};

export function useChannelProfilePanel({
  closeAgentSession,
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
    (pubkey: string) => {
      setOpenThreadHeadId(null);
      setExpandedThreadReplyIds(new Set());
      setThreadScrollTargetId(null);
      setThreadReplyTargetId(null);
      closeAgentSession();
      setChannelManagementOpen(false);
      setProfilePanelPubkey(pubkey);
    },
    [
      closeAgentSession,
      setChannelManagementOpen,
      setExpandedThreadReplyIds,
      setOpenThreadHeadId,
      setProfilePanelPubkey,
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
