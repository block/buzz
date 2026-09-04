import * as React from "react";

import type { MarkdownDocTarget } from "@/shared/ui/markdown/markdownDocViewerContext";

import { recordMarkdownDocOpener } from "./markdownDocFocus";

type UseChannelPaneOpenersOptions = {
  channelType: string | undefined;
  channelManagementOpen: boolean;
  closeAgentSession: () => void;
  openGlobalChannelManagement: () => void;
  openMarkdownDoc: (url: string, filename: string) => void;
  /** Prompts for an unresolved in-progress thread edit; false blocks the open. */
  requireThreadEditResolution: () => boolean;
  setChannelManagementOpen: (open: boolean) => void;
  setExpandedThreadReplyIds: React.Dispatch<React.SetStateAction<Set<string>>>;
  setOpenThreadHeadId: (id: string | null) => void;
  setProfilePanelPubkey: (pubkey: string | null) => void;
  setThreadReplyTargetId: (id: string | null) => void;
  setThreadScrollTargetId: (id: string | null) => void;
};

/**
 * Open handlers for ChannelPane's mutually exclusive auxiliary panes.
 *
 * Each opener clears every competing pane before opening its own (mirroring
 * useChannelProfilePanel) so a newly opened pane is never dead behind a
 * higher-priority sibling in ChannelPane's pane priority chain.
 */
export function useChannelPaneOpeners({
  channelType,
  channelManagementOpen,
  closeAgentSession,
  openGlobalChannelManagement,
  openMarkdownDoc,
  requireThreadEditResolution,
  setChannelManagementOpen,
  setExpandedThreadReplyIds,
  setOpenThreadHeadId,
  setProfilePanelPubkey,
  setThreadReplyTargetId,
  setThreadScrollTargetId,
}: UseChannelPaneOpenersOptions) {
  const clearCompetingPanes = React.useCallback(() => {
    setOpenThreadHeadId(null);
    setExpandedThreadReplyIds(new Set());
    setThreadScrollTargetId(null);
    setThreadReplyTargetId(null);
    closeAgentSession();
    setProfilePanelPubkey(null);
  }, [
    closeAgentSession,
    setExpandedThreadReplyIds,
    setOpenThreadHeadId,
    setProfilePanelPubkey,
    setThreadReplyTargetId,
    setThreadScrollTargetId,
  ]);

  const handleManageChannel = React.useCallback(() => {
    if (!requireThreadEditResolution()) return;
    if (channelType === "forum") {
      openGlobalChannelManagement();
      return;
    }
    if (channelManagementOpen) {
      setChannelManagementOpen(false);
      return;
    }
    clearCompetingPanes();
    setChannelManagementOpen(true);
  }, [
    channelType,
    channelManagementOpen,
    clearCompetingPanes,
    openGlobalChannelManagement,
    requireThreadEditResolution,
    setChannelManagementOpen,
  ]);

  const handleOpenMarkdownDoc = React.useCallback(
    (doc: MarkdownDocTarget) => {
      // Capture the invoking card's identity before a narrow-layout panel swap
      // can unmount it. In wide layouts an open thread remains mounted beneath
      // the document focus drawer, preserving its scroll and reply state.
      recordMarkdownDocOpener(doc.url, doc.opener ?? null);
      // A document opened from the center timeline takes the ordinary right
      // pane, replacing any open thread. A thread card (or URL-restored Inbox
      // context, which has no live opener element) preserves that thread so
      // ChannelPane can choose side-by-side vs stacked responsively.
      if (
        doc.opener &&
        !doc.opener.closest('[data-testid="message-thread-panel"]')
      ) {
        setOpenThreadHeadId(null);
        setExpandedThreadReplyIds(new Set());
        setThreadScrollTargetId(null);
        setThreadReplyTargetId(null);
      }
      closeAgentSession();
      setProfilePanelPubkey(null);
      setChannelManagementOpen(false);
      openMarkdownDoc(doc.url, doc.filename);
    },
    [
      closeAgentSession,
      openMarkdownDoc,
      setChannelManagementOpen,
      setExpandedThreadReplyIds,
      setOpenThreadHeadId,
      setProfilePanelPubkey,
      setThreadReplyTargetId,
      setThreadScrollTargetId,
    ],
  );

  return { handleManageChannel, handleOpenMarkdownDoc };
}
