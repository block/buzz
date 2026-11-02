// biome-ignore format: keep compact to stay within file size limit
import { useLocation } from "@tanstack/react-router";
import * as React from "react";
import { SidebarDndContext } from "@/features/sidebar/ui/SidebarDnd";

import { AddCommunityDialog } from "@/features/communities/ui/AddCommunityDialog";
import { useIsMobile } from "@/shared/hooks/use-mobile";
import { ColonyField } from "@/shared/ui/ColonyField";
import { PowerIcon, SettingsIcon } from "@/shared/ui/colonyIcons";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { useDeferredLoad } from "@/shared/hooks/useDeferredStartup";
import {
  useChannelSections,
  type ChannelSection,
} from "@/features/sidebar/lib/useChannelSections";
import { useActiveWorkingChannelsById } from "@/features/sidebar/lib/useActiveWorkingChannelsById";
import { useDmSidebarMetadata } from "@/features/sidebar/useDmSidebarMetadata";
import { sortDmChannelsForSidebar } from "@/features/sidebar/lib/dmSidebarSort";
import {
  sectionSortGroupKey,
  sortChannelsForSidebar,
} from "@/features/sidebar/lib/channelSortPreference";
import { useChannelSortPreference } from "@/features/sidebar/lib/useChannelSortPreference";
import { useSidebarScrollLock } from "@/features/sidebar/lib/useSidebarScrollLock";
import { isSidebarBackgroundTarget } from "@/features/sidebar/lib/sidebarBackgroundTarget";
import {
  sidebarOverflowUnreadLabel,
  useSidebarUnreadOverflow,
} from "@/features/sidebar/lib/useSidebarUnreadOverflow";
import {
  CreateSectionDialog,
  DeleteSectionAlertDialog,
  RenameSectionDialog,
  useDeleteChannelDialog,
  useLeaveChannelDialog,
  type SectionDialogValue,
} from "@/features/sidebar/ui/ChannelSectionDialogs";
import {
  AppSidebarPinnedHeader,
  AppSidebarPrimaryMenu,
} from "@/features/sidebar/ui/AppSidebarPinnedHeader";
import {
  canPreviewUnreadDm,
  MoreUnreadButton,
  preferredUnreadTarget,
} from "@/features/sidebar/ui/MoreUnreadButton";
import { SidebarSection } from "@/features/sidebar/ui/SidebarSection";
import { SidebarCollapsedNav } from "@/features/sidebar/ui/SidebarCollapsedNav";
import { SidebarDestinationList } from "@/features/sidebar/ui/SidebarDestinationList";
import type { SidebarDestinationContext } from "@/features/sidebar/ui/sidebarDestinations";
import {
  ChannelGroupSection,
  CustomChannelSection,
  SectionActionsMenu,
  SectionQuickAction,
} from "@/features/sidebar/ui/CustomChannelSection";
import { CreateChannelDialog } from "@/features/sidebar/ui/CreateChannelDialog";
import { SidebarProfileCard } from "@/features/sidebar/ui/SidebarProfileCard";
import type {
  AppSidebarProps,
  CollapsibleSidebarGroup,
  CreateChannelKind,
} from "@/features/sidebar/ui/AppSidebar.types";
import { SidebarRelayConnectionCard } from "@/features/sidebar/ui/SidebarRelayConnectionCard";
import { SidebarRelayErrorNotice } from "@/features/sidebar/ui/SidebarRelayErrorNotice";
import {
  SidebarLoadingContent,
  useSidebarLoadingShape,
} from "@/features/sidebar/ui/sidebarLoadingSkeleton";
import { useDeferredModalOpen } from "@/shared/ui/deferredModalOpen";
import { SidebarUpdateCard } from "@/features/settings/SidebarUpdateCard";
import { useUpdaterContext } from "@/features/settings/hooks/UpdaterProvider";
import { shouldShowSidebarUpdateCard } from "@/features/settings/sidebarUpdateCardVisibility";
import type { Channel, ChannelVisibility } from "@/shared/api/types";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarMenu,
  SidebarRail,
  useSidebar,
} from "@/shared/ui/sidebar";
import {
  ProtectedBestieSidebarEntry,
  useProtectedVisibleDirectMessages,
} from "@protected-feature-components";

export function AppSidebar({
  addCommunityPrefill,
  activeCommunity,
  channels,
  currentPubkey,
  fallbackDisplayName,
  homeBadgeCount,
  isPowerActive,
  workspaceDialogMode,
  onBackgroundClick,
  isAddCommunityOpen,
  isLoading,
  isCreatingChannel,
  profile,
  relayConnectionCard,
  selfPresenceStatus,
  showSidebarCollapseButton,
  errorMessage,
  selectedChannelId,
  suppressTodaySelection = false,
  selectedView,
  unreadChannelCounts,
  unreadChannelIds,
  highPriorityUnreadChannelIds,
  previewActivityChannelIds,
  communities,
  onAddCommunity,
  onAddCommunityOpenChange,
  onCreateChannel,
  onOpenWorkspaceDialog,
  onOpenWorkspaceSettings,
  onSendFeedback,
  onHideDm,
  onMarkChannelUnread,
  onMarkChannelRead,
  onMarkAllChannelsRead,
  onBrowseChannels,
  onOpenDm,
  onCreateAgent,
  onSelectToday,
  onSelectAgents,
  onSelectTeam,
  onSelectClients,
  onSelectWork,
  onSelectPower,
  onSelectHome,
  onSelectChannel,
  onOpenSearchResult,
  searchChannels,
  searchFocusRequests,
  onSelectSettings,
  onSetPresenceStatus,
  onSetUserStatus,
  onClearUserStatus,
  onSwitchCommunity,
  onRemoveWorkspace,
  selfUserStatus,
  isPresencePending,
  onNewMessage,
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
  const { open: sidebarOpen, openMobile, toggleSidebar } = useSidebar();
  const isMobile = useIsMobile();
  const isCollapsedRail = !isMobile && !sidebarOpen;
  const [isSidebarUpdateCardDismissed, setIsSidebarUpdateCardDismissed] =
    React.useState(false);
  const showSidebarUpdateCard =
    canShowSidebarUpdateCard && !isSidebarUpdateCardDismissed;
  const { pathname } = useLocation();
  const suppressActivitySelection = pathname.startsWith("/company/");
  const destinationContext = React.useMemo<SidebarDestinationContext>(
    () => ({
      handlers: {
        onSelectAgents,
        onSelectClients,
        onSelectSettings,
        onSelectTeam,
        onSelectWork,
      },
      pathname,
      selectedChannelId: selectedView === "channel" ? selectedChannelId : null,
      selectedView,
    }),
    [
      onSelectAgents,
      onSelectClients,
      onSelectSettings,
      onSelectTeam,
      onSelectWork,
      pathname,
      selectedChannelId,
      selectedView,
    ],
  );
  const [dmActionsMenuOpen, setDmActionsMenuOpen] = React.useState(false);
  const allDirectMessages = React.useMemo(
    () => channels.filter((channel) => channel.channelType === "dm"),
    [channels],
  );
  const directMessages = useProtectedVisibleDirectMessages(
    allDirectMessages,
    currentPubkey,
  );
  const dmChannelIds = React.useMemo(
    () => new Set(directMessages.map(({ id }) => id)),
    [directMessages],
  );
  const scrollRef = React.useRef<HTMLDivElement>(null);
  useSidebarScrollLock(scrollRef);
  // biome-ignore format: keep compact to stay within file size limit
  const { hasHighPriorityAbove, hasHighPriorityBelow, scrollToChannel, scrollToNextAbove, scrollToNextBelow, unreadAboveCount, unreadBelowCount, unreadMessageBelowChannelIds } = useSidebarUnreadOverflow({ dmChannelIds, highPriorityUnreadChannelIds, previewActivityChannelIds, scrollRef, unreadChannelIds });

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
  } = useChannelSections(currentPubkey, activeCommunity?.relayUrl);

  const sectionIds = React.useMemo(
    () => channelSections.map((s) => s.id),
    [channelSections],
  );

  const { sortModeFor, setSortModeFor } = useChannelSortPreference(
    currentPubkey,
    activeCommunity?.relayUrl,
    sectionIds,
  );

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
  const { requestDeleteChannel, dialog: deleteChannelDialog } =
    useDeleteChannelDialog((channel) => {
      if (channel.id === selectedChannelId) onSelectHome();
    });

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
    // Apply each grouping's own sort preference; section membership itself
    // is untouched.
    for (const sectionId of Object.keys(bySection)) {
      bySection[sectionId] = sortChannelsForSidebar(
        bySection[sectionId],
        sortModeFor(sectionSortGroupKey(sectionId)),
      );
    }
    return {
      bySection,
      unassigned: sortChannelsForSidebar(unassigned, sortModeFor("channels")),
    };
  }, [
    streamChannels,
    channelSections,
    channelAssignments,
    starredChannelIds,
    sortModeFor,
  ]);

  const starredChannels = React.useMemo(() => {
    if (!starredChannelIds || starredChannelIds.size === 0) return [];
    return sortChannelsForSidebar(
      streamChannels.filter((channel) => starredChannelIds.has(channel.id)),
      sortModeFor("starred"),
    );
  }, [streamChannels, starredChannelIds, sortModeFor]);

  const handleCreateSectionForChannel = React.useCallback(
    (channelId: string) => {
      setCreateSectionState({ open: true, pendingChannelId: channelId });
    },
    [],
  );

  const handleCreateChannelInSection = React.useCallback(
    (sectionId: string) => {
      onBrowseChannels?.((channelId) => assignChannel(channelId, sectionId));
    },
    [assignChannel, onBrowseChannels],
  );

  const handleCreateSectionConfirm = React.useCallback(
    (value: SectionDialogValue) => {
      const section = createSection(value.name, value.icon);
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
    () =>
      sortDmChannelsForSidebar(
        directMessages,
        dmChannelLabels,
        sortModeFor("dms"),
      ),
    [directMessages, dmChannelLabels, sortModeFor],
  );
  const unreadDmPreviewsBelow = React.useMemo(
    () =>
      unreadMessageBelowChannelIds.flatMap((channelId) => {
        const channel = directMessages.find(
          (candidate) => candidate.id === channelId,
        );
        const participants = dmParticipantsByChannelId[channelId];
        const participant = participants?.[0];
        if (
          !channel ||
          !participant ||
          !canPreviewUnreadDm(
            channel.participantPubkeys.length,
            participants?.length ?? 0,
          )
        ) {
          return [];
        }
        return [
          {
            accessibleLabel: participant.label,
            avatarUrl: participant.avatarUrl,
            channelId,
            isAgent: participant.isAgent,
            label: dmChannelLabels[channelId] ?? participant.label,
            pubkey: participant.pubkey,
          },
        ];
      }),
    [
      directMessages,
      dmChannelLabels,
      dmParticipantsByChannelId,
      unreadMessageBelowChannelIds,
    ],
  );
  const unreadDmChannelIds = React.useMemo(
    () => new Set(directMessages.map(({ id }) => id)),
    [directMessages],
  );
  const nextUnreadDmBelowId = preferredUnreadTarget(
    unreadMessageBelowChannelIds,
    unreadDmChannelIds,
  );
  const sidebarLoadingShape = useSidebarLoadingShape({
    activeCommunityId: activeCommunity?.id,
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
  const isCreatingAny = createDialogKind === "stream" && isCreatingChannel;

  const handleCreateFromDialog = React.useCallback(
    async (input: {
      name: string;
      description?: string;
      visibility: ChannelVisibility;
      ttlSeconds?: number;
    }) => {
      if (createDialogKind === "stream") {
        await onCreateChannel(input);
      }
    },
    [createDialogKind, onCreateChannel],
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
      className="app-sidebar-full-shell !z-[100] !border-r-0"
      collapsible="icon"
      data-colony-workspace-route={
        selectedView === "today" ||
        selectedView === "channel" ||
        selectedView === "pins"
          ? "true"
          : undefined
      }
      data-colony-full-app-shell="true"
      data-testid="app-sidebar"
      onClick={(event) => {
        if (isSidebarBackgroundTarget(event.target)) {
          onBackgroundClick?.();
        }
      }}
      variant="sidebar"
    >
      <div
        className="relative flex min-h-0 flex-1 flex-col overflow-hidden"
        data-sidebar-background
        data-testid="app-sidebar-scroll-anchor"
      >
        <ColonyField />
        <AppSidebarPinnedHeader
          activeWorkspace={activeCommunity}
          workspaces={communities}
          channelLabels={dmChannelLabels}
          currentPubkey={currentPubkey}
          currentChannelId={
            selectedView === "channel" ? selectedChannelId : null
          }
          onBrowseChannels={onBrowseChannels}
          onCopyWorkspaceLink={() => {
            if (activeCommunity) {
              copyTextToClipboard(
                activeCommunity.relayUrl,
                "Workspace link copied.",
              );
            }
          }}
          onCreateWorkspace={() => onOpenWorkspaceDialog("create")}
          onCreateAgent={onCreateAgent}
          onCreateChannel={handleOpenCreateChannel}
          onOpenDm={onOpenDm}
          onOpenSearchResult={onOpenSearchResult}
          onInvitePeople={() => onSelectSettings("community-members")}
          onJoinWorkspace={() => onOpenWorkspaceDialog("join")}
          onOpenWorkspaceSettings={onOpenWorkspaceSettings}
          onRemoveWorkspace={onRemoveWorkspace}
          onSelectChannel={onSelectChannel}
          onSwitchWorkspace={onSwitchCommunity}
          searchChannels={searchChannels}
          searchFocusRequest={searchFocusRequests[0]}
          showSidebarCollapseButton={showSidebarCollapseButton}
          scopeSearchFocusRequest={searchFocusRequests[1]}
          suggestionChannels={channels}
        />

        <div
          className="relative flex min-h-0 flex-1 flex-col"
          data-sidebar-background
          data-testid="sidebar-channel-content"
        >
          {unreadAboveCount > 0 ? (
            <MoreUnreadButton
              count={unreadAboveCount}
              emphasis={hasHighPriorityAbove ? "primary" : "default"}
              label={sidebarOverflowUnreadLabel(unreadAboveCount)}
              onClick={scrollToNextAbove}
              position="top"
              testId="sidebar-more-unread-above"
            />
          ) : null}

          <SidebarContent
            className="buzz-sidebar-scrollbar overscroll-none [overflow-anchor:none]"
            data-sidebar-background
            ref={scrollRef}
          >
            <div
              className="flex w-full flex-col gap-2 px-[3px]"
              data-sidebar-background
              data-testid="sidebar-scroll-content"
            >
              {!isCollapsedRail ? (
                <AppSidebarPrimaryMenu
                  homeBadgeCount={homeBadgeCount}
                  onSelectToday={onSelectToday}
                  onSelectHome={onSelectHome}
                  suppressActivitySelection={suppressActivitySelection}
                  suppressTodaySelection={suppressTodaySelection}
                  selectedView={selectedView}
                />
              ) : null}

              {isCollapsedRail ? (
                <SidebarCollapsedNav
                  destinationContext={destinationContext}
                  directMessages={sortedDirectMessages.map((channel) => ({
                    channel,
                    isActive:
                      selectedView === "channel" &&
                      selectedChannelId === channel.id,
                    label: dmChannelLabels[channel.id] ?? channel.name,
                    participants: dmParticipantsByChannelId[channel.id],
                    unreadCount: unreadChannelIds.has(channel.id)
                      ? Math.max(unreadChannelCounts.get(channel.id) ?? 0, 1)
                      : 0,
                  }))}
                  hasUnreadChannels={streamChannels.some((channel) =>
                    unreadChannelIds.has(channel.id),
                  )}
                  homeBadgeCount={homeBadgeCount}
                  isActivityActive={
                    selectedView === "home" && !suppressActivitySelection
                  }
                  isTodayActive={
                    selectedView === "today" && !suppressTodaySelection
                  }
                  onExpand={toggleSidebar}
                  onSelectChannel={onSelectChannel}
                  onSelectHome={onSelectHome}
                  onSelectToday={onSelectToday}
                />
              ) : null}

              {!isCollapsedRail && isLoading ? (
                <SidebarLoadingContent shape={sidebarLoadingShape} />
              ) : null}

              {!isCollapsedRail && !isLoading ? (
                <>
                  <div
                    className="colony-sidebar-nav"
                    data-testid="sidebar-nav-destinations"
                  >
                    <SidebarDestinationList context={destinationContext} />
                  </div>

                  <div
                    className="colony-sidebar-conversations"
                    data-testid="sidebar-nav-conversations"
                  >
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
                          key={section.id}
                          section={section}
                          channels={sectionBuckets.bySection[section.id] ?? []}
                          hasUnread={
                            sectionBuckets.bySection[section.id]?.some((c) =>
                              unreadChannelIds.has(c.id),
                            ) ?? false
                          }
                          isCollapsed={collapsedSections[section.id] ?? false}
                          isActiveChannel={selectedView === "channel"}
                          activeWorkingByChannelId={activeWorkingByChannelId}
                          selectedChannelId={selectedChannelId}
                          unreadChannelIds={unreadChannelIds}
                          unreadChannelCounts={unreadChannelCounts}
                          sections={channelSections}
                          assignments={channelAssignments}
                          isFirst={idx === 0}
                          isLast={idx === channelSections.length - 1}
                          sortMode={sortModeFor(
                            sectionSortGroupKey(section.id),
                          )}
                          onSortModeChange={(mode) =>
                            setSortModeFor(
                              sectionSortGroupKey(section.id),
                              mode,
                            )
                          }
                          onToggleCollapsed={() =>
                            toggleCollapsedSection(section.id)
                          }
                          onSelectChannel={onSelectChannel}
                          onMarkChannelRead={onMarkChannelRead}
                          onMarkChannelUnread={onMarkChannelUnread}
                          onMarkSectionRead={() => {
                            for (const channel of sectionBuckets.bySection[
                              section.id
                            ] ?? []) {
                              onMarkChannelRead(
                                channel.id,
                                channel.lastMessageAt,
                              );
                            }
                          }}
                          onAssignChannel={assignChannel}
                          onUnassignChannel={unassignChannel}
                          onCreateSectionForChannel={
                            handleCreateSectionForChannel
                          }
                          onCreateChannel={() =>
                            handleCreateChannelInSection(section.id)
                          }
                          onRenameSection={() =>
                            setRenameSectionTarget(section)
                          }
                          onDeleteSection={() =>
                            setDeleteSectionTarget(section)
                          }
                          onMoveSectionUp={() => moveSectionUp(section.id)}
                          onMoveSectionDown={() => moveSectionDown(section.id)}
                          mutedChannelIds={mutedChannelIds}
                          onMuteChannel={onMuteChannel}
                          onUnmuteChannel={onUnmuteChannel}
                          starredChannelIds={starredChannelIds}
                          onStarChannel={onStarChannel}
                          onUnstarChannel={onUnstarChannel}
                          onDeleteChannel={requestDeleteChannel}
                          onLeaveChannel={requestLeaveChannel}
                        />
                      ))}
                      <ChannelGroupSection
                        draggable
                        hasUnread={unreadChannelIds.size > 0}
                        isCollapsed={collapsedGroups.channels}
                        isActiveChannel={selectedView === "channel"}
                        activeWorkingByChannelId={activeWorkingByChannelId}
                        items={sectionBuckets.unassigned}
                        sortMode={sortModeFor("channels")}
                        onSortModeChange={(mode) =>
                          setSortModeFor("channels", mode)
                        }
                        actionsTestId="section-actions-channels"
                        listTestId="stream-list"
                        onBrowseClick={() => onBrowseChannels?.()}
                        onQuickCreateClick={() => onBrowseChannels?.()}
                        quickCreateLabel="Browse channels"
                        showQuickCreate={Boolean(onBrowseChannels)}
                        browseLabel="Browse channels"
                        onMarkAllRead={onMarkAllChannelsRead}
                        onMarkChannelRead={onMarkChannelRead}
                        onMarkChannelUnread={onMarkChannelUnread}
                        onSelectChannel={onSelectChannel}
                        onToggleCollapsed={() =>
                          toggleCollapsedGroup("channels")
                        }
                        selectedChannelId={selectedChannelId}
                        title="Channels"
                        unreadChannelIds={unreadChannelIds}
                        unreadChannelCounts={unreadChannelCounts}
                        sections={channelSections}
                        assignments={channelAssignments}
                        onAssignChannel={assignChannel}
                        onUnassignChannel={unassignChannel}
                        onCreateSectionForChannel={
                          handleCreateSectionForChannel
                        }
                        mutedChannelIds={mutedChannelIds}
                        onMuteChannel={onMuteChannel}
                        onUnmuteChannel={onUnmuteChannel}
                        starredChannelIds={starredChannelIds}
                        onStarChannel={onStarChannel}
                        onUnstarChannel={onUnstarChannel}
                        onDeleteChannel={requestDeleteChannel}
                        onLeaveChannel={requestLeaveChannel}
                      />
                    </SidebarDndContext>

                    {starredChannels.length > 0 ? (
                      <ChannelGroupSection
                        hasUnread={starredChannels.some((c) =>
                          unreadChannelIds.has(c.id),
                        )}
                        isCollapsed={collapsedGroups.starred}
                        isActiveChannel={selectedView === "channel"}
                        activeWorkingByChannelId={activeWorkingByChannelId}
                        items={starredChannels}
                        sortMode={sortModeFor("starred")}
                        onSortModeChange={(mode) =>
                          setSortModeFor("starred", mode)
                        }
                        actionsTestId="section-actions-starred"
                        listTestId="starred-list"
                        onMarkAllRead={() => {
                          for (const channel of starredChannels) {
                            onMarkChannelRead(
                              channel.id,
                              channel.lastMessageAt,
                            );
                          }
                        }}
                        onMarkChannelRead={onMarkChannelRead}
                        onMarkChannelUnread={onMarkChannelUnread}
                        onSelectChannel={onSelectChannel}
                        onToggleCollapsed={() =>
                          toggleCollapsedGroup("starred")
                        }
                        selectedChannelId={selectedChannelId}
                        title="Starred"
                        unreadChannelIds={unreadChannelIds}
                        unreadChannelCounts={unreadChannelCounts}
                        mutedChannelIds={mutedChannelIds}
                        onMuteChannel={onMuteChannel}
                        onUnmuteChannel={onUnmuteChannel}
                        starredChannelIds={starredChannelIds}
                        onStarChannel={onStarChannel}
                        onUnstarChannel={onUnstarChannel}
                        onDeleteChannel={requestDeleteChannel}
                        onLeaveChannel={requestLeaveChannel}
                      />
                    ) : null}

                    <SidebarSection
                      action={
                        <div className="absolute right-1 top-1/2 z-10 flex -translate-y-1/2 items-center gap-0.5">
                          <SectionQuickAction
                            label="New message"
                            onClick={onNewMessage}
                            testId="section-actions-dms-quick-create"
                          />
                          <SectionActionsMenu
                            sectionLabel="direct messages"
                            testId="section-actions-dms"
                            onOpenChange={setDmActionsMenuOpen}
                            onNewMessage={onNewMessage}
                            sortMode={sortModeFor("dms")}
                            onSortModeChange={(mode) =>
                              setSortModeFor("dms", mode)
                            }
                          />
                        </div>
                      }
                      dmParticipantsByChannelId={dmParticipantsByChannelId}
                      isCollapsed={collapsedGroups.directMessages}
                      isActiveChannel={selectedView === "channel"}
                      activeWorkingByChannelId={activeWorkingByChannelId}
                      items={sortedDirectMessages}
                      channelLabels={dmChannelLabels}
                      onHideDm={onHideDm}
                      onMarkChannelRead={onMarkChannelRead}
                      onMarkChannelUnread={onMarkChannelUnread}
                      onSelectChannel={onSelectChannel}
                      onToggleCollapsed={() =>
                        toggleCollapsedGroup("directMessages")
                      }
                      presenceByChannelId={dmPresenceByChannelId}
                      selectedChannelId={selectedChannelId}
                      testId="dm-list"
                      title="Direct messages"
                      sectionActionsOpen={dmActionsMenuOpen}
                      unreadChannelCounts={unreadChannelCounts}
                      unreadChannelIds={unreadChannelIds}
                      mutedChannelIds={mutedChannelIds}
                      onMuteChannel={onMuteChannel}
                      onUnmuteChannel={onUnmuteChannel}
                    />
                    <SidebarMenu>
                      <ProtectedBestieSidebarEntry />
                    </SidebarMenu>
                  </div>

                  {errorMessage &&
                  !relayConnectionCard.hasRelayUnreachableError ? (
                    <SidebarRelayErrorNotice message={errorMessage} />
                  ) : null}
                </>
              ) : null}
            </div>
          </SidebarContent>
        </div>

        <div className="relative z-30 shrink-0" data-buzz-glass-footer-wrap>
          {unreadBelowCount > 0 ? (
            <MoreUnreadButton
              bottomClassName="bottom-full"
              count={unreadBelowCount}
              dmPreviews={unreadDmPreviewsBelow}
              emphasis={hasHighPriorityBelow ? "primary" : "default"}
              label={sidebarOverflowUnreadLabel(unreadBelowCount)}
              onClick={() =>
                nextUnreadDmBelowId
                  ? scrollToChannel(nextUnreadDmBelowId)
                  : scrollToNextBelow()
              }
              position="bottom"
              targetChannelId={nextUnreadDmBelowId}
              testId="sidebar-more-unread-below"
            />
          ) : null}

          <SidebarFooter>
            {relayConnectionCard.showSidebarRelayConnectionCard &&
            (isMobile ? openMobile : sidebarOpen) ? (
              <SidebarRelayConnectionCard
                className="mb-2"
                isConnected={relayConnectionCard.isRelayConnectionSuccess}
                isReconnectPending={relayConnectionCard.isRelayReconnectPending}
                isWaitingOnReconnectHook={
                  relayConnectionCard.isWaitingOnReconnectHook
                }
                onDismiss={relayConnectionCard.onDismissRelayConnectionCard}
                onReconnect={relayConnectionCard.onReconnectRelay}
              />
            ) : null}
            {showSidebarUpdateCard ? (
              <div className="mb-2 group-data-[collapsible=icon]:hidden">
                <SidebarUpdateCard
                  onDismiss={() => setIsSidebarUpdateCardDismissed(true)}
                />
              </div>
            ) : null}
            <div className="colony-sidebar-footer-row">
              <SidebarProfileCard
                isPresencePending={isPresencePending}
                onOpenSettings={onSelectSettings}
                onSendFeedback={onSendFeedback}
                onSetPresenceStatus={onSetPresenceStatus}
                onSetUserStatus={onSetUserStatus}
                onClearUserStatus={onClearUserStatus}
                profile={profile}
                resolvedDisplayName={resolvedDisplayName}
                selfPresenceStatus={selfPresenceStatus}
                selfUserStatus={selfUserStatus}
              />
              <button
                aria-label="AI spend and power"
                className="colony-sidebar-footer-icon"
                data-active={isPowerActive || undefined}
                data-testid="sidebar-ai-spend-power"
                onClick={onSelectPower}
                title="AI spend & power"
                type="button"
              >
                <PowerIcon size={18} />
              </button>
              <button
                aria-label="Settings"
                className="colony-sidebar-footer-icon"
                data-testid="sidebar-settings"
                onClick={() => onSelectSettings()}
                title="Settings"
                type="button"
              >
                <SettingsIcon size={18} />
              </button>
            </div>
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

      <AddCommunityDialog
        mode={workspaceDialogMode}
        onOpenWorkspaceDialog={onOpenWorkspaceDialog}
        prefill={addCommunityPrefill}
        onOpenChange={onAddCommunityOpenChange ?? (() => {})}
        onSubmit={onAddCommunity}
        open={isAddCommunityOpen ?? false}
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
        sectionIcon={renameSectionTarget?.icon}
        onConfirm={(value) => {
          if (renameSectionTarget) {
            renameSection(renameSectionTarget.id, value.name, value.icon);
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
      {deleteChannelDialog}
      {leaveChannelDialog}
      <SidebarRail />
    </Sidebar>
  );
}
