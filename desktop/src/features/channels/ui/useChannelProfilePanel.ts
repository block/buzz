import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useOpenDmMutation } from "@/features/channels/hooks";
import {
  captureRestorableScrollAnchor,
  type RestorableScrollAnchor,
} from "@/features/messages/lib/restorableScrollAnchor";
import { usePanelReturnTarget } from "@/shared/hooks/usePanelReturnTarget";

type UseChannelProfilePanelOptions = {
  activeChannelId: string | null;
  closeAgentSession: () => void;
  openThreadHeadId: string | null;
  profilePanelPubkey: string | null;
  setChannelManagementOpen: (open: boolean) => void;
  setExpandedThreadReplyIds: (value: Set<string>) => void;
  setOpenThreadHeadId: (value: string | null) => void;
  setProfilePanelPubkey: (value: string | null) => void;
  setThreadReplyTargetId: (value: string | null) => void;
  setThreadScrollTargetId: (value: string | null) => void;
};

type ProfilePanelReturnTarget = {
  scrollAnchor: RestorableScrollAnchor | null;
  threadHeadId: string;
};

export function useChannelProfilePanel({
  activeChannelId,
  closeAgentSession,
  openThreadHeadId,
  profilePanelPubkey,
  setChannelManagementOpen,
  setExpandedThreadReplyIds,
  setOpenThreadHeadId,
  setProfilePanelPubkey,
  setThreadReplyTargetId,
  setThreadScrollTargetId,
}: UseChannelProfilePanelOptions) {
  const { goChannel } = useAppNavigation();
  const openDmMutation = useOpenDmMutation();
  const [threadInitialScrollAnchor, setThreadInitialScrollAnchor] =
    React.useState<RestorableScrollAnchor | null>(null);
  const { hasTarget: hasProfilePanelReturnTarget, store: returnTarget } =
    usePanelReturnTarget<ProfilePanelReturnTarget>(activeChannelId);

  const clearThreadPanelState = React.useCallback(() => {
    setExpandedThreadReplyIds(new Set());
    setThreadInitialScrollAnchor(null);
    setThreadScrollTargetId(null);
    setThreadReplyTargetId(null);
  }, [
    setExpandedThreadReplyIds,
    setThreadReplyTargetId,
    setThreadScrollTargetId,
  ]);

  const handleOpenProfilePanel = React.useCallback(
    (pubkey: string) => {
      const replacingThread =
        profilePanelPubkey === null && openThreadHeadId !== null;
      if (profilePanelPubkey === null) {
        const scrollAnchor = replacingThread
          ? captureRestorableScrollAnchor(
              document.querySelector<HTMLDivElement>(
                '[data-testid="message-thread-body"]',
              ),
            )
          : null;
        returnTarget.capture(
          replacingThread
            ? {
                scrollAnchor,
                threadHeadId: openThreadHeadId,
              }
            : null,
        );
      }
      setOpenThreadHeadId(null);
      setThreadScrollTargetId(null);
      if (!replacingThread && profilePanelPubkey === null) {
        clearThreadPanelState();
      }
      closeAgentSession();
      setChannelManagementOpen(false);
      setProfilePanelPubkey(pubkey);
    },
    [
      clearThreadPanelState,
      closeAgentSession,
      openThreadHeadId,
      profilePanelPubkey,
      returnTarget,
      setChannelManagementOpen,
      setOpenThreadHeadId,
      setProfilePanelPubkey,
      setThreadScrollTargetId,
    ],
  );

  const handleCloseProfilePanel = React.useCallback(() => {
    returnTarget.clear();
    clearThreadPanelState();
    setProfilePanelPubkey(null);
  }, [clearThreadPanelState, returnTarget, setProfilePanelPubkey]);

  const handleBackFromProfilePanel = React.useCallback(() => {
    const target = returnTarget.consume();
    setProfilePanelPubkey(null);
    if (!target) {
      clearThreadPanelState();
      return;
    }

    setThreadInitialScrollAnchor(target.scrollAnchor);
    setOpenThreadHeadId(target.threadHeadId);
  }, [
    clearThreadPanelState,
    returnTarget,
    setOpenThreadHeadId,
    setProfilePanelPubkey,
  ]);

  const handleThreadInitialScrollAnchorRestored = React.useCallback(() => {
    setThreadInitialScrollAnchor(null);
  }, []);

  const openDmMutateAsync = openDmMutation.mutateAsync;
  const handleOpenDm = React.useCallback(
    async (pubkeys: string[]) => {
      const dm = await openDmMutateAsync({ pubkeys });
      await goChannel(dm.id);
    },
    [goChannel, openDmMutateAsync],
  );

  return {
    handleBackFromProfilePanel,
    handleOpenProfilePanel,
    handleCloseProfilePanel,
    handleOpenDm,
    handleThreadInitialScrollAnchorRestored,
    hasProfilePanelReturnTarget,
    threadInitialScrollAnchor,
  };
}
