import * as React from "react";
import type { TimelineMessage } from "@/features/messages/types";

type UseChannelPanelActionsOptions = {
  activeChannelType?: string | null;
  channelManagementOpen: boolean;
  channelPanelOpen: boolean;
  handleCloseAgentSession: () => void;
  handleOpenAgentSession: (pubkey: string, channelId?: string | null) => void;
  handleOpenProfilePanel: (pubkey: string) => void;
  handleOpenThreadAndCloseAgentSession: (message: TimelineMessage) => void;
  history: {
    setChannelManagementOpen: (open: boolean) => void;
    setChannelPanelOpen: (open: boolean) => void;
    setOpenThreadHeadId: (id: string | null) => void;
    setProfilePanelPubkey: (pubkey: string | null) => void;
  };
  openGlobalChannelManagement: () => void;
  setExpandedThreadReplyIds: (ids: Set<string>) => void;
  setThreadReplyTargetId: (id: string | null) => void;
  setThreadScrollTargetId: (id: string | null) => void;
};

export function useChannelPanelActions({
  activeChannelType,
  channelManagementOpen,
  channelPanelOpen,
  handleCloseAgentSession,
  handleOpenAgentSession,
  handleOpenProfilePanel,
  handleOpenThreadAndCloseAgentSession,
  history,
  openGlobalChannelManagement,
  setExpandedThreadReplyIds,
  setThreadReplyTargetId,
  setThreadScrollTargetId,
}: UseChannelPanelActionsOptions) {
  const {
    setChannelManagementOpen,
    setChannelPanelOpen,
    setOpenThreadHeadId,
    setProfilePanelPubkey,
  } = history;
  const handleOpenChannelPanel = React.useCallback(() => {
    if (channelPanelOpen) {
      setChannelPanelOpen(false);
      return;
    }

    setChannelManagementOpen(false);
    setOpenThreadHeadId(null);
    setExpandedThreadReplyIds(new Set());
    setThreadScrollTargetId(null);
    setThreadReplyTargetId(null);
    handleCloseAgentSession();
    setProfilePanelPubkey(null);
    setChannelPanelOpen(true);
  }, [
    channelPanelOpen,
    handleCloseAgentSession,
    setChannelManagementOpen,
    setChannelPanelOpen,
    setExpandedThreadReplyIds,
    setOpenThreadHeadId,
    setProfilePanelPubkey,
    setThreadReplyTargetId,
    setThreadScrollTargetId,
  ]);

  const handleOpenThreadWithPanelClosed = React.useCallback(
    (message: TimelineMessage) => {
      setChannelPanelOpen(false);
      handleOpenThreadAndCloseAgentSession(message);
    },
    [handleOpenThreadAndCloseAgentSession, setChannelPanelOpen],
  );
  const handleOpenAgentSessionWithPanelClosed = React.useCallback(
    (pubkey: string, channelId?: string | null) => {
      setChannelPanelOpen(false);
      handleOpenAgentSession(pubkey, channelId);
    },
    [handleOpenAgentSession, setChannelPanelOpen],
  );
  const handleOpenProfilePanelWithPanelClosed = React.useCallback(
    (pubkey: string) => {
      setChannelPanelOpen(false);
      handleOpenProfilePanel(pubkey);
    },
    [handleOpenProfilePanel, setChannelPanelOpen],
  );
  const handleManageChannel = React.useCallback(() => {
    if (activeChannelType === "forum") {
      openGlobalChannelManagement();
      return;
    }

    setChannelPanelOpen(false);
    if (channelManagementOpen) {
      setChannelManagementOpen(false);
      return;
    }

    setOpenThreadHeadId(null);
    setExpandedThreadReplyIds(new Set());
    setThreadScrollTargetId(null);
    setThreadReplyTargetId(null);
    handleCloseAgentSession();
    setProfilePanelPubkey(null);
    setChannelManagementOpen(true);
  }, [
    activeChannelType,
    channelManagementOpen,
    handleCloseAgentSession,
    openGlobalChannelManagement,
    setChannelManagementOpen,
    setChannelPanelOpen,
    setExpandedThreadReplyIds,
    setOpenThreadHeadId,
    setProfilePanelPubkey,
    setThreadReplyTargetId,
    setThreadScrollTargetId,
  ]);

  return {
    handleManageChannel,
    handleOpenAgentSessionWithPanelClosed,
    handleOpenChannelPanel,
    handleOpenProfilePanelWithPanelClosed,
    handleOpenThreadWithPanelClosed,
  };
}
