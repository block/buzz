// biome-ignore format: keep compact to stay within file size limit
import {
  Activity,
  Bot,
  FolderGit2,
  Home,
  Plus,
  Zap,
} from "lucide-react";
import * as React from "react";
import { AnimatePresence } from "motion/react";
import { FeatureGate } from "@/shared/features";
import { SidebarDndContext } from "@/features/sidebar/ui/SidebarDnd";

import type { Workspace } from "@/features/workspaces/types";
import { AddWorkspaceDialog } from "@/features/workspaces/ui/AddWorkspaceDialog";
import { useDeferredLoad } from "@/shared/hooks/useDeferredStartup";
import {
  useChannelSections,
  type ChannelSection,
} from "@/features/sidebar/lib/useChannelSections";
import { useDmSidebarMetadata } from "@/features/sidebar/useDmSidebarMetadata";
import { useSidebarScrollLock } from "@/features/sidebar/lib/useSidebarScrollLock";
import { useUnreadOverflow } from "@/features/sidebar/lib/useUnreadOverflow";
import {
  CreateSectionDialog,
  DeleteSectionAlertDialog,
  RenameSectionDialog,
} from "@/features/sidebar/ui/ChannelSectionDialogs";
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
import { SidebarUpdateCard } from "@/features/settings/SidebarUpdateCard";
import { useUpdaterContext } from "@/features/settings/hooks/UpdaterProvider";
import { shouldShowSidebarUpdateCard } from "@/features/settings/sidebarUpdateCardVisibility";
import type {
  Channel,
  ChannelVisibility,
  PresenceStatus,
  Profile,
  UserStatus,
} from "@/shared/api/types";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarRail,
} from "@/shared/ui/sidebar";

type CollapsibleSidebarGroup =
  | "starred"
  | "channels"
  | "forums"
  | "directMessages";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type CreateChannelKind = "stream" | "forum";

type AppSidebarProps = {
  activeWorkspace: Workspace | null;
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
  onSelectAgents: () => void;
  onSelectProjects: () => void;
  onSelectPulse: () => void;
  onSelectWorkflows: () => void;
  onSelectHome: () => void;
  onSelectChannel: (channelId: string) => void;
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

// ---------------------------------------------------------------------------
// AppSidebar
// ---------------------------------------------------------------------------

export function AppSidebar({
  activeWorkspace,
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
  onHideDm,
  onMarkChannelUnread,
  onMarkChannelRead,
  onMarkAllChannelsRead,
  onBrowseChannels,
  onOpenDm,
  onUpdateWorkspace,
  onRemoveWorkspace,
  onSelectAgents,
  onSelectProjects,
  onSelectPulse,
  onSelectWorkflows,
  onSelectHome,
  onSelectChannel,
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
      setCreateDialogKind("stream");
    }
  }, [isCreateChannelOpenProp]);
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
  } = useUnreadOverflow({ scrollRef, unreadChannelIds });

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
        {unreadAboveCount > 0 ? (
          <MoreUnreadButton
            count={unreadAboveCount}
            onClick={scrollToNextAbove}
            position="top"
            testId="sidebar-more-unread-above"
          />
        ) : null}
        <SidebarContent
          className="buzz-sidebar-scrollbar mt-(--buzz-top-chrome-height,2.5rem) overscroll-none"
          ref={scrollRef}
        >
          <SidebarHeader
            className="cursor-default select-none"
            data-tauri-drag-region
          >
            <SidebarMenu>
              <SidebarMenuItem>
                <SidebarMenuButton
                  isActive={selectedView === "home"}
                  onClick={onSelectHome}
                  tooltip="Home"
                  type="button"
                >
                  <Home className="h-4 w-4" />
                  <span>Home</span>
                </SidebarMenuButton>
                {homeBadgeCount > 0 ? (
                  <SidebarMenuBadge
                    className="right-2 rounded-full bg-primary/15 px-1.5 text-2xs text-primary peer-data-[active=true]/menu-button:bg-sidebar-active-foreground/20 peer-data-[active=true]/menu-button:text-sidebar-active-foreground"
                    data-testid="sidebar-home-count"
                  >
                    {Math.min(homeBadgeCount, 99)}
                  </SidebarMenuBadge>
                ) : null}
              </SidebarMenuItem>
              <FeatureGate feature="pulse">
                <SidebarMenuItem>
                  <SidebarMenuButton
                    data-testid="open-pulse-view"
                    isActive={selectedView === "pulse"}
                    onClick={onSelectPulse}
                    tooltip="Pulse"
                    type="button"
                  >
                    <Activity className="h-4 w-4" />
                    <span>Pulse</span>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              </FeatureGate>
              <FeatureGate feature="projects">
                <SidebarMenuItem>
                  <SidebarMenuButton
                    data-testid="open-projects-view"
                    isActive={selectedView === "projects"}
                    onClick={onSelectProjects}
                    tooltip="Projects"
                    type="button"
                  >
                    <FolderGit2 className="h-4 w-4" />
                    <span>Projects</span>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              </FeatureGate>
              <SidebarMenuItem>
                <SidebarMenuButton
                  data-testid="open-agents-view"
                  isActive={selectedView === "agents"}
                  onClick={onSelectAgents}
                  tooltip="Agents"
                  type="button"
                >
                  <Bot className="h-4 w-4" />
                  <span>Agents</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
              <FeatureGate feature="workflows">
                <SidebarMenuItem>
                  <SidebarMenuButton
                    data-testid="open-workflows-view"
                    isActive={selectedView === "workflows"}
                    onClick={onSelectWorkflows}
                    tooltip="Workflows"
                    type="button"
                  >
                    <Zap className="h-4 w-4" />
                    <span>Workflows</span>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              </FeatureGate>
            </SidebarMenu>
          </SidebarHeader>

          {isLoading ? (
            <SidebarLoadingContent shape={sidebarLoadingShape} />
          ) : null}

          {!isLoading ? (
            <>
              {starredChannels.length > 0 ? (
                <ChannelGroupSection
                  createAriaLabel="Starred channels"
                  hasUnread={starredChannels.some((c) =>
                    unreadChannelIds.has(c.id),
                  )}
                  isCollapsed={collapsedGroups.starred}
                  isActiveChannel={selectedView === "channel"}
                  items={starredChannels}
                  listTestId="starred-list"
                  onMarkAllRead={() => {
                    for (const channel of starredChannels) {
                      onMarkChannelRead(channel.id, channel.lastMessageAt);
                    }
                  }}
                  onMarkChannelRead={onMarkChannelRead}
                  onMarkChannelUnread={onMarkChannelUnread}
                  onSelectChannel={onSelectChannel}
                  onToggleCollapsed={() => toggleCollapsedGroup("starred")}
                  selectedChannelId={selectedChannelId}
                  title="Starred"
                  unreadChannelCounts={unreadChannelCounts}
                  unreadChannelIds={unreadChannelIds}
                  mutedChannelIds={mutedChannelIds}
                  onMuteChannel={onMuteChannel}
                  onUnmuteChannel={onUnmuteChannel}
                  starredChannelIds={starredChannelIds}
                  onStarChannel={onStarChannel}
                  onUnstarChannel={onUnstarChannel}
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
                    selectedChannelId={selectedChannelId}
                    unreadChannelCounts={unreadChannelCounts}
                    unreadChannelIds={unreadChannelIds}
                    sections={channelSections}
                    assignments={channelAssignments}
                    isFirst={idx === 0}
                    isLast={idx === channelSections.length - 1}
                    onToggleCollapsed={() => toggleCollapsedSection(section.id)}
                    onSelectChannel={onSelectChannel}
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
                  />
                ))}
                <ChannelGroupSection
                  browseAriaLabel="Browse channels"
                  createAriaLabel="Create a channel"
                  draggable
                  groupClassName={
                    channelSections.length > 0 ? undefined : "pt-1"
                  }
                  hasUnread={unreadChannelIds.size > 0}
                  isCollapsed={collapsedGroups.channels}
                  isActiveChannel={selectedView === "channel"}
                  items={sectionBuckets.unassigned}
                  listTestId="stream-list"
                  onBrowseClick={onBrowseChannels}
                  onCreateClick={() => setCreateDialogKind("stream")}
                  onMarkAllRead={onMarkAllChannelsRead}
                  onMarkChannelRead={onMarkChannelRead}
                  onMarkChannelUnread={onMarkChannelUnread}
                  onSelectChannel={onSelectChannel}
                  onToggleCollapsed={() => toggleCollapsedGroup("channels")}
                  selectedChannelId={selectedChannelId}
                  title="Channels"
                  unreadChannelCounts={unreadChannelCounts}
                  unreadChannelIds={unreadChannelIds}
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
                />
              </SidebarDndContext>
              <FeatureGate feature="forum">
                <ChannelGroupSection
                  createAriaLabel="Create a forum"
                  hasUnread={unreadChannelIds.size > 0}
                  isCollapsed={collapsedGroups.forums}
                  isActiveChannel={selectedView === "channel"}
                  items={forumChannels}
                  listTestId="forum-list"
                  onCreateClick={() => setCreateDialogKind("forum")}
                  onMarkAllRead={onMarkAllChannelsRead}
                  onMarkChannelRead={onMarkChannelRead}
                  onMarkChannelUnread={onMarkChannelUnread}
                  onSelectChannel={onSelectChannel}
                  onToggleCollapsed={() => toggleCollapsedGroup("forums")}
                  selectedChannelId={selectedChannelId}
                  title="Forums"
                  unreadChannelCounts={unreadChannelCounts}
                  unreadChannelIds={unreadChannelIds}
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
                      <Plus className="h-4 w-4" />
                    </button>
                  </div>
                }
                dmParticipantsByChannelId={dmParticipantsByChannelId}
                isCollapsed={collapsedGroups.directMessages}
                isActiveChannel={selectedView === "channel"}
                items={directMessages}
                channelLabels={dmChannelLabels}
                onHideDm={onHideDm}
                onMarkChannelRead={onMarkChannelRead}
                onMarkChannelUnread={onMarkChannelUnread}
                onSelectChannel={onSelectChannel}
                onToggleCollapsed={() => toggleCollapsedGroup("directMessages")}
                presenceByChannelId={dmPresenceByChannelId}
                selectedChannelId={selectedChannelId}
                testId="dm-list"
                title="Direct Messages"
                unreadChannelCounts={unreadChannelCounts}
                unreadChannelIds={unreadChannelIds}
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
      <SidebarRail />
    </Sidebar>
  );
}
