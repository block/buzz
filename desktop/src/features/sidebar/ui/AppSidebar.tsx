// biome-ignore format: keep compact to stay within file size limit
import { MessageCirclePlus } from "lucide-react";
import * as React from "react";
import { AnimatePresence } from "motion/react";
import { FeatureGate } from "@/shared/features";
import { SidebarDndContext } from "@/features/sidebar/ui/SidebarDnd";

import type { Workspace } from "@/features/workspaces/types";
import type { AgentConversation } from "@/features/agents/agentConversations";
import { AddWorkspaceDialog } from "@/features/workspaces/ui/AddWorkspaceDialog";
import { useDeferredLoad } from "@/shared/hooks/useDeferredStartup";
import {
  useChannelSections,
  type ChannelSection,
} from "@/features/sidebar/lib/useChannelSections";
import { useActiveWorkingChannelsById } from "@/features/sidebar/lib/useActiveWorkingChannelsById";
import { useDmSidebarMetadata } from "@/features/sidebar/useDmSidebarMetadata";
import { sortDmChannelsByLabel } from "@/features/sidebar/lib/dmSidebarSort";
import { useSidebarScrollLock } from "@/features/sidebar/lib/useSidebarScrollLock";
import { useUnreadOverflow } from "@/features/sidebar/lib/useUnreadOverflow";
import {
  CreateSectionDialog,
  DeleteSectionAlertDialog,
  RenameSectionDialog,
  useLeaveChannelDialog,
} from "@/features/sidebar/ui/ChannelSectionDialogs";
import { AppSidebarPinnedHeader } from "@/features/sidebar/ui/AppSidebarPinnedHeader";
import { MoreUnreadButton } from "@/features/sidebar/ui/MoreUnreadButton";
import { SidebarSection } from "@/features/sidebar/ui/SidebarSection";
import {
  ChannelGroupSection,
  CustomChannelSection,
} from "@/features/sidebar/ui/CustomChannelSection";
import { CreateChannelDialog } from "@/features/sidebar/ui/CreateChannelDialog";
import { NewDirectMessageDialog } from "@/features/sidebar/ui/NewDirectMessageDialog";
import { SidebarProfileCard } from "@/features/sidebar/ui/SidebarProfileCard";
import { SidebarRelayConnectionCard } from "@/features/sidebar/ui/SidebarRelayConnectionCard";
import { useSidebarRelayConnectionCard } from "@/features/sidebar/ui/useSidebarRelayConnectionCard";
import {
  SidebarLoadingContent,
  useSidebarLoadingShape,
} from "@/features/sidebar/ui/sidebarLoadingSkeleton";
import {
  SECTION_ACTION_VISIBILITY_CLASS,
  SECTION_ICON_BUTTON_CLASS,
} from "@/features/sidebar/ui/sidebarSectionStyles";
import { useDeferredModalOpen } from "@/shared/ui/deferredModalOpen";
import { SidebarUpdateCard } from "@/features/settings/SidebarUpdateCard";
import { useUpdaterContext } from "@/features/settings/hooks/UpdaterProvider";
import { shouldShowSidebarUpdateCard } from "@/features/settings/sidebarUpdateCardVisibility";
import type {
  Channel,
  ChannelVisibility,
  PresenceStatus,
  Profile,
  SearchHit,
  UserStatus,
} from "@/shared/api/types";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarMenu,
  SidebarMenuItem,
  SidebarRail,
  useSidebar,
} from "@/shared/ui/sidebar";

type CollapsibleSidebarGroup =
  | "starred"
  | "channels"
  | "forums"
  | "directMessages";

type CreateChannelKind = "stream" | "forum";

type AppSidebarProps = {
  activeWorkspace: Workspace | null;
  agentConversations?: AgentConversation[];
  channels: Channel[];
  currentPubkey?: string;
  fallbackDisplayName?: string;
  homeBadgeCount: number;
  isAddWorkspaceOpen?: boolean;
  isLoading: boolean;
  isCreatingChannel: boolean;
  isCreatingForum: boolean;
  isOpeningDm: boolean;
  profile?: Profile;
  selfPresenceStatus: PresenceStatus;
  errorMessage?: string;
  selectedAgentConversationId?: string | null;
  selectedChannelId: string | null;
  selectedView:
    | "home"
    | "channel"
    | "agents"
    | "workflows"
    | "pulse"
    | "projects";
  unreadChannelCounts: ReadonlyMap<string, number>;
  unreadChannelIds: ReadonlySet<string>;
  workspaces: Workspace[];
  onAddWorkspace: (workspace: Workspace) => void;
  onAddWorkspaceOpenChange?: (open: boolean) => void;
  onCreateChannel: (input: {
    name: string;
    description?: string;
    visibility: ChannelVisibility;
    ttlSeconds?: number;
    templateId?: string;
  }) => Promise<void>;
  onCreateForum: (input: {
    name: string;
    description?: string;
    visibility: ChannelVisibility;
    ttlSeconds?: number;
    templateId?: string;
  }) => Promise<void>;
  onOpenAddWorkspace: () => void;
  onHideAgentConversation?: (conversationId: string) => void;
  onHideDm: (channelId: string) => void;
  onMarkChannelUnread: (channelId: string) => void;
  onMarkChannelRead: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onMarkAllChannelsRead: () => void;
  onBrowseChannels?: () => void;
  onOpenDm: (input: { pubkeys: string[] }) => Promise<void>;
  onUpdateWorkspace: (
    id: string,
    updates: Partial<Pick<Workspace, "name" | "relayUrl" | "token">>,
  ) => void;
  onRemoveWorkspace: (id: string) => void;
  onCreateAgent: () => void;
  onSelectAgentConversation?: (conversationId: string) => void;
  onSelectAgents: () => void;
  onSelectProjects: () => void;
  onSelectPulse: () => void;
  onSelectWorkflows: () => void;
  onSelectHome: () => void;
  onSelectChannel: (channelId: string) => void;
  onOpenSearchResult: (hit: SearchHit) => void;
  /**
   * Full channel set used for global search. Unlike `channels` (which is
   * scoped to the viewer's joined sidebar list), this includes open channels
   * the viewer hasn't joined, so search can surface them.
   */
  searchChannels: Channel[];
  searchFocusRequest: number;
  onSelectSettings: (section?: "profile" | "appearance") => void;
  onSetPresenceStatus?: (status: "online" | "away" | "offline") => void;
  onSetUserStatus: (text: string, emoji: string) => void;
  onClearUserStatus: () => void;
  onSwitchWorkspace: (id: string) => void;
  selfUserStatus?: UserStatus;
  isPresencePending?: boolean;
  isNewDmOpen?: boolean;
  onNewDmOpenChange?: (open: boolean) => void;
  isCreateChannelOpen?: boolean;
  onCreateChannelOpenChange?: (open: boolean) => void;
  mutedChannelIds?: ReadonlySet<string>;
  onMuteChannel?: (channelId: string) => void;
  onUnmuteChannel?: (channelId: string) => void;
  starredChannelIds?: ReadonlySet<string>;
  onStarChannel?: (channelId: string) => void;
  onUnstarChannel?: (channelId: string) => void;
};

export function AppSidebar({
  activeWorkspace,
  agentConversations = [],
  channels,
  currentPubkey,
  fallbackDisplayName,
  homeBadgeCount,
  isAddWorkspaceOpen,
  isLoading,
  isCreatingChannel,
  isCreatingForum,
  isOpeningDm,
  profile,
  selfPresenceStatus,
  errorMessage,
  selectedAgentConversationId,
  selectedChannelId,
  selectedView,
  unreadChannelCounts,
  unreadChannelIds,
  workspaces,
  onAddWorkspace,
  onAddWorkspaceOpenChange,
  onCreateChannel,
  onCreateForum,
  onOpenAddWorkspace,
  onHideAgentConversation,
  onHideDm,
  onMarkChannelUnread,
  onMarkChannelRead,
  onMarkAllChannelsRead,
  onBrowseChannels,
  onOpenDm,
  onUpdateWorkspace,
  onRemoveWorkspace,
  onCreateAgent,
  onSelectAgentConversation,
  onSelectAgents,
  onSelectProjects,
  onSelectPulse,
  onSelectWorkflows,
  onSelectHome,
  onSelectChannel,
  onOpenSearchResult,
  searchChannels,
  searchFocusRequest,
  onSelectSettings,
  onSetPresenceStatus,
  onSetUserStatus,
  onClearUserStatus,
  onSwitchWorkspace,
  selfUserStatus,
  isPresencePending,
  isNewDmOpen: isNewDmOpenProp,
  onNewDmOpenChange,
  isCreateChannelOpen: isCreateChannelOpenProp,
  onCreateChannelOpenChange,
  mutedChannelIds,
  onMuteChannel,
  onUnmuteChannel,
  starredChannelIds,
  onStarChannel,
  onUnstarChannel,
}: AppSidebarProps) {
  const activeWorkingByChannelId = useActiveWorkingChannelsById();
  const { status: updateStatus } = useUpdaterContext();
  const canShowSidebarUpdateCard = shouldShowSidebarUpdateCard(updateStatus);
  const sidebarRelayConnectionCard = useSidebarRelayConnectionCard(
    errorMessage,
    activeWorkspace?.relayUrl,
  );
  const [isSidebarUpdateCardDismissed, setIsSidebarUpdateCardDismissed] =
    React.useState(false);
  const showSidebarUpdateCard =
    canShowSidebarUpdateCard && !isSidebarUpdateCardDismissed;
  const [isNewDmOpenInternal, setIsNewDmOpenInternal] = React.useState(false);
  const isNewDmOpen = isNewDmOpenProp ?? isNewDmOpenInternal;
  const setIsNewDmOpen = onNewDmOpenChange ?? setIsNewDmOpenInternal;
  const scrollRef = React.useRef<HTMLDivElement>(null);
  useSidebarScrollLock(scrollRef);

  // Search lives in the sidebar's pinned header, so a ⌘K focus request must
  // first reveal the sidebar. When collapsed (offcanvas) on desktop the input
  // is mounted but translated off-screen; on mobile it is unmounted entirely.
  // Open the sidebar on each focus-request bump so the input the shortcut
  // focuses is actually visible. Skips the initial mount (request === 0).
  const { isMobile, setOpen, setOpenMobile } = useSidebar();
  React.useEffect(() => {
    if (searchFocusRequest === 0) {
      return;
    }

    if (isMobile) {
      setOpenMobile(true);
    } else {
      setOpen(true);
    }
  }, [searchFocusRequest, isMobile, setOpen, setOpenMobile]);

  React.useEffect(() => {
    const scrollElement = scrollRef.current;
    if (!scrollElement) return;

    const handleWheel = (event: WheelEvent) => {
      if (event.deltaY === 0) return;

      const maxScrollTop =
        scrollElement.scrollHeight - scrollElement.clientHeight;
      if (maxScrollTop <= 0) {
        event.preventDefault();
        event.stopPropagation();
        return;
      }

      const atTop = scrollElement.scrollTop <= 0;
      const atBottom = scrollElement.scrollTop >= maxScrollTop - 1;
      const scrollingPastTop = event.deltaY < 0 && atTop;
      const scrollingPastBottom = event.deltaY > 0 && atBottom;

      if (scrollingPastTop || scrollingPastBottom) {
        event.preventDefault();
        event.stopPropagation();
        scrollElement.scrollTop = scrollingPastTop ? 0 : maxScrollTop;
      }
    };

    scrollElement.addEventListener("wheel", handleWheel, {
      capture: true,
      passive: false,
    });
    return () => {
      scrollElement.removeEventListener("wheel", handleWheel, {
        capture: true,
      });
    };
  }, []);

  const [createDialogKind, setCreateDialogKind] =
    React.useState<CreateChannelKind | null>(null);
  const { openNextFrame: openModalNextFrame } = useDeferredModalOpen();
  const openCreateDialog = React.useCallback(
    (kind: CreateChannelKind) => {
      setCreateDialogKind(null);
      openModalNextFrame(() => setCreateDialogKind(kind));
    },
    [openModalNextFrame],
  );

  React.useEffect(() => {
    if (!canShowSidebarUpdateCard) {
      setIsSidebarUpdateCardDismissed(false);
    }
  }, [canShowSidebarUpdateCard]);

  // Allow the create-channel dialog to be opened from outside (e.g. the
  // ⌘⇧N global shortcut in AppShell), mirroring the controlled new-DM lift.
  // When the external flag flips on, open the "stream" create dialog; the
  // close direction is reported back via `onCreateChannelOpenChange` in the
  // dialog's `onOpenChange` below.
  React.useEffect(() => {
    if (isCreateChannelOpenProp) {
      openCreateDialog("stream");
    }
  }, [isCreateChannelOpenProp, openCreateDialog]);
  const [collapsedGroups, setCollapsedGroups] = React.useState<
    Record<CollapsibleSidebarGroup, boolean>
  >({
    starred: false,
    channels: false,
    forums: false,
    directMessages: false,
  });

  const toggleCollapsedGroup = React.useCallback(
    (group: CollapsibleSidebarGroup) => {
      setCollapsedGroups((current) => ({
        ...current,
        [group]: !current[group],
      }));
    },
    [],
  );

  const [collapsedSections, setCollapsedSections] = React.useState<
    Record<string, boolean>
  >({});
  const toggleCollapsedSection = React.useCallback((sectionId: string) => {
    setCollapsedSections((current) => ({
      ...current,
      [sectionId]: !current[sectionId],
    }));
  }, []);

  const {
    sections: channelSections,
    assignments: channelAssignments,
    createSection,
    renameSection,
    deleteSection,
    moveSectionUp,
    moveSectionDown,
    reorderSections,
    assignChannel,
    unassignChannel,
  } = useChannelSections(currentPubkey);

  const [createSectionState, setCreateSectionState] = React.useState<{
    open: boolean;
    pendingChannelId: string | null;
  }>({ open: false, pendingChannelId: null });
  const [renameSectionTarget, setRenameSectionTarget] =
    React.useState<ChannelSection | null>(null);
  const [deleteSectionTarget, setDeleteSectionTarget] =
    React.useState<ChannelSection | null>(null);
  const { requestLeaveChannel, dialog: leaveChannelDialog } =
    useLeaveChannelDialog();

  const sectionIds = React.useMemo(
    () => channelSections.map((s) => s.id),
    [channelSections],
  );

  const streamChannels = React.useMemo(
    () => channels.filter((channel) => channel.channelType === "stream"),
    [channels],
  );

  const sectionBuckets = React.useMemo(() => {
    const bySection: Record<string, Channel[]> = {};
    const unassigned: Channel[] = [];
    const sectionIds = new Set(channelSections.map((s) => s.id));

    for (const channel of streamChannels) {
      if (starredChannelIds?.has(channel.id)) continue;
      const sectionId = channelAssignments[channel.id];
      if (sectionId && sectionIds.has(sectionId)) {
        if (!bySection[sectionId]) {
          bySection[sectionId] = [];
        }
        bySection[sectionId].push(channel);
      } else {
        unassigned.push(channel);
      }
    }
    return { bySection, unassigned };
  }, [streamChannels, channelSections, channelAssignments, starredChannelIds]);

  const starredChannels = React.useMemo(() => {
    if (!starredChannelIds || starredChannelIds.size === 0) return [];
    return streamChannels.filter((channel) =>
      starredChannelIds.has(channel.id),
    );
  }, [streamChannels, starredChannelIds]);

  const handleCreateSectionForChannel = React.useCallback(
    (channelId: string) => {
      setCreateSectionState({ open: true, pendingChannelId: channelId });
    },
    [],
  );

  const handleCreateSectionConfirm = React.useCallback(
    (name: string) => {
      const section = createSection(name);
      if (!section) {
        return;
      }
      if (createSectionState.pendingChannelId) {
        assignChannel(createSectionState.pendingChannelId, section.id);
      }
      setCreateSectionState({ open: false, pendingChannelId: null });
    },
    [createSection, assignChannel, createSectionState.pendingChannelId],
  );

  const forumChannels = React.useMemo(
    () => channels.filter((channel) => channel.channelType === "forum"),
    [channels],
  );
  const directMessages = React.useMemo(
    () => channels.filter((channel) => channel.channelType === "dm"),
    [channels],
  );
  const isSelectedDirectMessage =
    selectedView === "channel" &&
    directMessages.some((channel) => channel.id === selectedChannelId);
  const shouldLoadDmMetadata = useDeferredLoad({
    immediate: isSelectedDirectMessage,
    timeoutMs: 400,
  });
  const { dmChannelLabels, dmParticipantsByChannelId, dmPresenceByChannelId } =
    useDmSidebarMetadata({
      currentPubkey,
      directMessages,
      enabled: shouldLoadDmMetadata,
      fallbackDisplayName,
      profileDisplayName: profile?.displayName,
    });
  const sortedDirectMessages = React.useMemo(
    () => sortDmChannelsByLabel(directMessages, dmChannelLabels),
    [directMessages, dmChannelLabels],
  );
  const agentConversationsByChannelId = React.useMemo(() => {
    const byChannelId = new Map<string, AgentConversation[]>();

    for (const conversation of agentConversations) {
      const channelConversations =
        byChannelId.get(conversation.channelId) ?? [];
      channelConversations.push(conversation);
      byChannelId.set(conversation.channelId, channelConversations);
    }

    return byChannelId;
  }, [agentConversations]);
  const isAgentConversationActive = selectedView === "agents";
  const displayUnreadChannelIds = unreadChannelIds;
  const displayUnreadChannelCounts = unreadChannelCounts;
  const sidebarLoadingShape = useSidebarLoadingShape({
    activeWorkspaceId: activeWorkspace?.id,
    currentPubkey,
    directMessages,
    dmChannelLabels,
    isLoading,
    streamChannels,
  });
  const resolvedDisplayName =
    profile?.displayName?.trim() ||
    fallbackDisplayName?.trim() ||
    "Current identity";
  const {
    scrollToNextAbove,
    scrollToNextBelow,
    unreadAboveCount,
    unreadBelowCount,
  } = useUnreadOverflow({
    scrollRef,
    unreadChannelIds: displayUnreadChannelIds,
  });

  const isCreatingAny =
    createDialogKind === "stream"
      ? isCreatingChannel
      : createDialogKind === "forum"
        ? isCreatingForum
        : false;

  const handleCreateFromDialog = React.useCallback(
    async (input: {
      name: string;
      description?: string;
      visibility: ChannelVisibility;
      ttlSeconds?: number;
      templateId?: string;
    }) => {
      if (createDialogKind === "stream") {
        await onCreateChannel(input);
      } else if (createDialogKind === "forum") {
        await onCreateForum(input);
      }
    },
    [createDialogKind, onCreateChannel, onCreateForum],
  );

  const handleOpenCreateChannel = React.useCallback(() => {
    if (onCreateChannelOpenChange) {
      onCreateChannelOpenChange(true);
      return;
    }

    openCreateDialog("stream");
  }, [onCreateChannelOpenChange, openCreateDialog]);

  return (
    <Sidebar
      className="!border-r-0"
      collapsible="offcanvas"
      data-testid="app-sidebar"
      variant="sidebar"
    >
      <div
        className="relative flex min-h-0 flex-1 flex-col overflow-hidden"
        data-testid="app-sidebar-scroll-anchor"
      >
        <AppSidebarPinnedHeader
          channelLabels={dmChannelLabels}
          currentPubkey={currentPubkey}
          homeBadgeCount={homeBadgeCount}
          onCreateAgent={onCreateAgent}
          onCreateChannel={handleOpenCreateChannel}
          onOpenDm={onOpenDm}
          onOpenSearchResult={onOpenSearchResult}
          onSelectAgents={onSelectAgents}
          onSelectChannel={onSelectChannel}
          onSelectHome={onSelectHome}
          onSelectProjects={onSelectProjects}
          onSelectPulse={onSelectPulse}
          onSelectWorkflows={onSelectWorkflows}
          searchChannels={searchChannels}
          searchFocusRequest={searchFocusRequest}
          selectedView={selectedView}
          suggestionChannels={channels}
        />

        <div
          className="relative flex min-h-0 flex-1 flex-col"
          data-testid="sidebar-channel-content"
        >
          {unreadAboveCount > 0 ? (
            <MoreUnreadButton
              count={unreadAboveCount}
              onClick={scrollToNextAbove}
              position="top"
              testId="sidebar-more-unread-above"
            />
          ) : null}

          <SidebarContent
            className="buzz-sidebar-scrollbar overscroll-none pt-4"
            ref={scrollRef}
          >
            {isLoading ? (
              <SidebarLoadingContent shape={sidebarLoadingShape} />
            ) : null}

            {!isLoading ? (
              <>
                {starredChannels.length > 0 ? (
                  <ChannelGroupSection
                    agentConversationsByChannelId={
                      agentConversationsByChannelId
                    }
                    createAriaLabel="Starred channels"
                    hasUnread={starredChannels.some((c) =>
                      displayUnreadChannelIds.has(c.id),
                    )}
                    isAgentConversationActive={isAgentConversationActive}
                    isCollapsed={collapsedGroups.starred}
                    isActiveChannel={selectedView === "channel"}
                    activeWorkingByChannelId={activeWorkingByChannelId}
                    items={starredChannels}
                    listTestId="starred-list"
                    onMarkAllRead={() => {
                      for (const channel of starredChannels) {
                        onMarkChannelRead(channel.id, channel.lastMessageAt);
                      }
                    }}
                    onHideAgentConversation={onHideAgentConversation}
                    onMarkChannelRead={onMarkChannelRead}
                    onMarkChannelUnread={onMarkChannelUnread}
                    onSelectAgentConversation={onSelectAgentConversation}
                    onSelectChannel={onSelectChannel}
                    onToggleCollapsed={() => toggleCollapsedGroup("starred")}
                    selectedChannelId={selectedChannelId}
                    selectedAgentConversationId={selectedAgentConversationId}
                    title="Starred"
                    unreadChannelCounts={displayUnreadChannelCounts}
                    unreadChannelIds={displayUnreadChannelIds}
                    mutedChannelIds={mutedChannelIds}
                    onMuteChannel={onMuteChannel}
                    onUnmuteChannel={onUnmuteChannel}
                    starredChannelIds={starredChannelIds}
                    onStarChannel={onStarChannel}
                    onUnstarChannel={onUnstarChannel}
                    onLeaveChannel={requestLeaveChannel}
                  />
                ) : null}
                <SidebarDndContext
                  channels={channels}
                  sections={channelSections}
                  sectionIds={sectionIds}
                  onAssignChannel={assignChannel}
                  onUnassignChannel={unassignChannel}
                  onReorderSections={reorderSections}
                >
                  {channelSections.map((section, idx) => (
                    <CustomChannelSection
                      agentConversationsByChannelId={
                        agentConversationsByChannelId
                      }
                      key={section.id}
                      section={section}
                      channels={sectionBuckets.bySection[section.id] ?? []}
                      hasUnread={
                        sectionBuckets.bySection[section.id]?.some((c) =>
                          displayUnreadChannelIds.has(c.id),
                        ) ?? false
                      }
                      isAgentConversationActive={isAgentConversationActive}
                      isCollapsed={collapsedSections[section.id] ?? false}
                      isActiveChannel={selectedView === "channel"}
                      activeWorkingByChannelId={activeWorkingByChannelId}
                      selectedChannelId={selectedChannelId}
                      selectedAgentConversationId={selectedAgentConversationId}
                      unreadChannelCounts={displayUnreadChannelCounts}
                      unreadChannelIds={displayUnreadChannelIds}
                      sections={channelSections}
                      assignments={channelAssignments}
                      isFirst={idx === 0}
                      isLast={idx === channelSections.length - 1}
                      onToggleCollapsed={() =>
                        toggleCollapsedSection(section.id)
                      }
                      onHideAgentConversation={onHideAgentConversation}
                      onSelectChannel={onSelectChannel}
                      onSelectAgentConversation={onSelectAgentConversation}
                      onMarkChannelRead={onMarkChannelRead}
                      onMarkChannelUnread={onMarkChannelUnread}
                      onMarkSectionRead={() => {
                        for (const channel of sectionBuckets.bySection[
                          section.id
                        ] ?? []) {
                          onMarkChannelRead(channel.id, channel.lastMessageAt);
                        }
                      }}
                      onAssignChannel={assignChannel}
                      onUnassignChannel={unassignChannel}
                      onCreateSectionForChannel={handleCreateSectionForChannel}
                      onRenameSection={() => setRenameSectionTarget(section)}
                      onDeleteSection={() => setDeleteSectionTarget(section)}
                      onMoveSectionUp={() => moveSectionUp(section.id)}
                      onMoveSectionDown={() => moveSectionDown(section.id)}
                      mutedChannelIds={mutedChannelIds}
                      onMuteChannel={onMuteChannel}
                      onUnmuteChannel={onUnmuteChannel}
                      starredChannelIds={starredChannelIds}
                      onStarChannel={onStarChannel}
                      onUnstarChannel={onUnstarChannel}
                      onLeaveChannel={requestLeaveChannel}
                    />
                  ))}
                  <ChannelGroupSection
                    agentConversationsByChannelId={
                      agentConversationsByChannelId
                    }
                    browseAriaLabel="Browse channels"
                    createAriaLabel="Create a channel"
                    draggable
                    hasUnread={displayUnreadChannelIds.size > 0}
                    isAgentConversationActive={isAgentConversationActive}
                    isCollapsed={collapsedGroups.channels}
                    isActiveChannel={selectedView === "channel"}
                    activeWorkingByChannelId={activeWorkingByChannelId}
                    items={sectionBuckets.unassigned}
                    listTestId="stream-list"
                    onBrowseClick={onBrowseChannels}
                    onCreateClick={() => openCreateDialog("stream")}
                    onMarkAllRead={onMarkAllChannelsRead}
                    onHideAgentConversation={onHideAgentConversation}
                    onMarkChannelRead={onMarkChannelRead}
                    onMarkChannelUnread={onMarkChannelUnread}
                    onSelectAgentConversation={onSelectAgentConversation}
                    onSelectChannel={onSelectChannel}
                    onToggleCollapsed={() => toggleCollapsedGroup("channels")}
                    selectedChannelId={selectedChannelId}
                    selectedAgentConversationId={selectedAgentConversationId}
                    title="Channels"
                    unreadChannelCounts={displayUnreadChannelCounts}
                    unreadChannelIds={displayUnreadChannelIds}
                    sections={channelSections}
                    assignments={channelAssignments}
                    onAssignChannel={assignChannel}
                    onUnassignChannel={unassignChannel}
                    onCreateSectionForChannel={handleCreateSectionForChannel}
                    mutedChannelIds={mutedChannelIds}
                    onMuteChannel={onMuteChannel}
                    onUnmuteChannel={onUnmuteChannel}
                    starredChannelIds={starredChannelIds}
                    onStarChannel={onStarChannel}
                    onUnstarChannel={onUnstarChannel}
                    onLeaveChannel={requestLeaveChannel}
                  />
                </SidebarDndContext>
                <FeatureGate feature="forum">
                  <ChannelGroupSection
                    agentConversationsByChannelId={
                      agentConversationsByChannelId
                    }
                    createAriaLabel="Create a forum"
                    hasUnread={displayUnreadChannelIds.size > 0}
                    isAgentConversationActive={isAgentConversationActive}
                    isCollapsed={collapsedGroups.forums}
                    isActiveChannel={selectedView === "channel"}
                    activeWorkingByChannelId={activeWorkingByChannelId}
                    items={forumChannels}
                    listTestId="forum-list"
                    onCreateClick={() => openCreateDialog("forum")}
                    onMarkAllRead={onMarkAllChannelsRead}
                    onHideAgentConversation={onHideAgentConversation}
                    onMarkChannelRead={onMarkChannelRead}
                    onMarkChannelUnread={onMarkChannelUnread}
                    onSelectAgentConversation={onSelectAgentConversation}
                    onSelectChannel={onSelectChannel}
                    onToggleCollapsed={() => toggleCollapsedGroup("forums")}
                    selectedChannelId={selectedChannelId}
                    selectedAgentConversationId={selectedAgentConversationId}
                    title="Forums"
                    unreadChannelCounts={displayUnreadChannelCounts}
                    unreadChannelIds={displayUnreadChannelIds}
                    mutedChannelIds={mutedChannelIds}
                    onMuteChannel={onMuteChannel}
                    onUnmuteChannel={onUnmuteChannel}
                  />
                </FeatureGate>
                <SidebarSection
                  action={
                    <div className="absolute right-1 top-1/2 z-10 flex -translate-y-1/2 items-center gap-0.5">
                      <button
                        aria-expanded={isNewDmOpen}
                        aria-label="Compose new message"
                        className={`${SECTION_ICON_BUTTON_CLASS} ${SECTION_ACTION_VISIBILITY_CLASS}`}
                        data-testid="new-dm-trigger"
                        onClick={() => {
                          setIsNewDmOpen(true);
                        }}
                        title="Compose new message"
                        type="button"
                      >
                        <MessageCirclePlus className="h-4 w-4" />
                      </button>
                    </div>
                  }
                  agentConversationsByChannelId={agentConversationsByChannelId}
                  dmParticipantsByChannelId={dmParticipantsByChannelId}
                  isCollapsed={collapsedGroups.directMessages}
                  isAgentConversationActive={isAgentConversationActive}
                  isActiveChannel={selectedView === "channel"}
                  activeWorkingByChannelId={activeWorkingByChannelId}
                  items={sortedDirectMessages}
                  channelLabels={dmChannelLabels}
                  onHideAgentConversation={onHideAgentConversation}
                  onHideDm={onHideDm}
                  onMarkChannelRead={onMarkChannelRead}
                  onMarkChannelUnread={onMarkChannelUnread}
                  onSelectAgentConversation={onSelectAgentConversation}
                  onSelectChannel={onSelectChannel}
                  onToggleCollapsed={() =>
                    toggleCollapsedGroup("directMessages")
                  }
                  presenceByChannelId={dmPresenceByChannelId}
                  selectedAgentConversationId={selectedAgentConversationId}
                  selectedChannelId={selectedChannelId}
                  testId="dm-list"
                  title="Direct messages"
                  unreadChannelCounts={displayUnreadChannelCounts}
                  unreadChannelIds={displayUnreadChannelIds}
                  mutedChannelIds={mutedChannelIds}
                  onMuteChannel={onMuteChannel}
                  onUnmuteChannel={onUnmuteChannel}
                />
              </>
            ) : null}

            {errorMessage &&
            !sidebarRelayConnectionCard.hasRelayUnreachableError ? (
              <div className="px-3 py-2 text-sm text-destructive">
                {errorMessage}
              </div>
            ) : null}
          </SidebarContent>
        </div>

        <div className="relative z-30 shrink-0">
          {unreadBelowCount > 0 ? (
            <MoreUnreadButton
              bottomClassName="bottom-full"
              count={unreadBelowCount}
              onClick={scrollToNextBelow}
              position="bottom"
              testId="sidebar-more-unread-below"
            />
          ) : null}

          <SidebarFooter className="bg-sidebar/55 backdrop-blur-xl supports-[backdrop-filter]:bg-sidebar/45 dark:bg-sidebar/45 dark:supports-[backdrop-filter]:bg-sidebar/35">
            <AnimatePresence>
              {sidebarRelayConnectionCard.showSidebarRelayConnectionCard ? (
                <SidebarRelayConnectionCard
                  className="mb-2 group-data-[collapsible=icon]:hidden"
                  isConnected={
                    sidebarRelayConnectionCard.isRelayConnectionSuccess
                  }
                  isReconnectPending={
                    sidebarRelayConnectionCard.isRelayReconnectPending
                  }
                  onDismiss={
                    sidebarRelayConnectionCard.onDismissRelayConnectionCard
                  }
                  onReconnect={sidebarRelayConnectionCard.onReconnectRelay}
                  key="sidebar-relay-connection-card"
                />
              ) : null}
            </AnimatePresence>
            {showSidebarUpdateCard ? (
              <div className="mb-2 group-data-[collapsible=icon]:hidden">
                <SidebarUpdateCard
                  onDismiss={() => setIsSidebarUpdateCardDismissed(true)}
                />
              </div>
            ) : null}
            <SidebarMenu>
              <SidebarMenuItem>
                <SidebarProfileCard
                  activeWorkspace={activeWorkspace}
                  isPresencePending={isPresencePending}
                  onOpenAddWorkspace={onOpenAddWorkspace}
                  onOpenSettings={onSelectSettings}
                  onRemoveWorkspace={onRemoveWorkspace}
                  onSetPresenceStatus={onSetPresenceStatus}
                  onSetUserStatus={onSetUserStatus}
                  onClearUserStatus={onClearUserStatus}
                  onSwitchWorkspace={onSwitchWorkspace}
                  onUpdateWorkspace={onUpdateWorkspace}
                  profile={profile}
                  resolvedDisplayName={resolvedDisplayName}
                  selfPresenceStatus={selfPresenceStatus}
                  selfUserStatus={selfUserStatus}
                  workspaces={workspaces}
                />
              </SidebarMenuItem>
            </SidebarMenu>
          </SidebarFooter>
        </div>
      </div>

      <CreateChannelDialog
        channelKind={createDialogKind}
        isCreating={isCreatingAny}
        onOpenChange={(open) => {
          if (!open) {
            // If a "stream" dialog driven by the external controller is
            // closing, report it back so AppShell's open state resets.
            if (createDialogKind === "stream") {
              onCreateChannelOpenChange?.(false);
            }
            setCreateDialogKind(null);
          }
        }}
        onCreate={handleCreateFromDialog}
      />

      <NewDirectMessageDialog
        currentPubkey={currentPubkey}
        isPending={isOpeningDm}
        onOpenChange={setIsNewDmOpen}
        onSubmit={onOpenDm}
        open={isNewDmOpen}
      />

      <AddWorkspaceDialog
        onOpenChange={onAddWorkspaceOpenChange ?? (() => {})}
        onSubmit={onAddWorkspace}
        open={isAddWorkspaceOpen ?? false}
      />

      <CreateSectionDialog
        open={createSectionState.open}
        onOpenChange={(open) => {
          if (!open) {
            setCreateSectionState({ open: false, pendingChannelId: null });
          }
        }}
        onConfirm={handleCreateSectionConfirm}
      />

      <RenameSectionDialog
        open={renameSectionTarget !== null}
        onOpenChange={(open) => {
          if (!open) setRenameSectionTarget(null);
        }}
        sectionName={renameSectionTarget?.name ?? ""}
        onConfirm={(newName) => {
          if (renameSectionTarget) {
            renameSection(renameSectionTarget.id, newName);
          }
          setRenameSectionTarget(null);
        }}
      />

      <DeleteSectionAlertDialog
        open={deleteSectionTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteSectionTarget(null);
        }}
        sectionName={deleteSectionTarget?.name ?? ""}
        channelCount={
          deleteSectionTarget
            ? (sectionBuckets.bySection[deleteSectionTarget.id]?.length ?? 0)
            : 0
        }
        onConfirm={() => {
          if (deleteSectionTarget) {
            deleteSection(deleteSectionTarget.id);
            setCollapsedSections((prev) => {
              const next = { ...prev };
              delete next[deleteSectionTarget.id];
              return next;
            });
          }
          setDeleteSectionTarget(null);
        }}
      />
      {leaveChannelDialog}
      <SidebarRail />
    </Sidebar>
  );
}
