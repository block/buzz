// biome-ignore format: keep compact to stay within file size limit
import {
  Activity,
  ArrowDown,
  ArrowUp,
  Bot,
  FolderGit2,
  Home,
  PenSquare,
  Zap,
} from "lucide-react";
import * as React from "react";
import { SidebarDndContext } from "@/features/sidebar/ui/SidebarDnd";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import type { Workspace } from "@/features/workspaces/types";
import { AddWorkspaceDialog } from "@/features/workspaces/ui/AddWorkspaceDialog";
import { WorkspaceSwitcher } from "@/features/workspaces/ui/WorkspaceSwitcher";
import { useDeferredLoad } from "@/shared/hooks/useDeferredStartup";
import { getPresenceLabel } from "@/features/presence/lib/presence";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { ProfilePopover } from "@/features/profile/ui/ProfilePopover";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
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
  SECTION_ACTION_VISIBILITY_CLASS,
} from "@/features/sidebar/ui/CustomChannelSection";
import { CreateChannelDialog } from "@/features/sidebar/ui/CreateChannelDialog";
import { NewDirectMessageDialog } from "@/features/sidebar/ui/NewDirectMessageDialog";
import type {
  Channel,
  ChannelVisibility,
  PresenceStatus,
  Profile,
  UserStatus,
} from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupAction,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSkeleton,
} from "@/shared/ui/sidebar";

type CollapsibleSidebarGroup = "channels" | "forums" | "directMessages";

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
  unreadChannelIds: Set<string>;
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
  onOpenBrowseChannels: () => void;
  onOpenBrowseForums: () => void;
  onHideDm: (channelId: string) => void;
  onMarkChannelUnread: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onMarkChannelRead: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onMarkAllChannelsRead: () => void;
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
  onSelectSettings: () => void;
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
  unreadChannelIds,
  workspaces,
  onAddWorkspace,
  onAddWorkspaceOpenChange,
  onCreateChannel,
  onCreateForum,
  onOpenAddWorkspace,
  onOpenBrowseChannels,
  onOpenBrowseForums,
  onHideDm,
  onMarkChannelUnread,
  onMarkChannelRead,
  onMarkAllChannelsRead,
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
}: AppSidebarProps) {
  const skeletonRows = ["first", "second", "third", "fourth", "fifth", "sixth"];
  const [isNewDmOpenInternal, setIsNewDmOpenInternal] = React.useState(false);
  const isNewDmOpen = isNewDmOpenProp ?? isNewDmOpenInternal;
  const setIsNewDmOpen = onNewDmOpenChange ?? setIsNewDmOpenInternal;
  const scrollRef = React.useRef<HTMLDivElement>(null);
  useSidebarScrollLock(scrollRef);
  const [profilePopoverOpen, setProfilePopoverOpen] = React.useState(false);
  const [createDialogKind, setCreateDialogKind] =
    React.useState<CreateChannelKind | null>(null);

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
  }, [streamChannels, channelSections, channelAssignments]);

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
  const shouldLoadAgentCount = useDeferredLoad({
    immediate: selectedView === "agents",
    timeoutMs: 250,
  });
  const managedAgentsQuery = useManagedAgentsQuery({
    enabled: shouldLoadAgentCount,
  });
  const totalAgentCount = managedAgentsQuery.data?.length ?? 0;
  const shouldShowAgentCount =
    totalAgentCount > 0 || managedAgentsQuery.isFetched;
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
      <SidebarHeader
        className="cursor-default select-none pt-11"
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
                className="right-2 rounded-full bg-primary/15 px-1.5 text-[11px] text-primary peer-data-[active=true]/menu-button:bg-sidebar-primary-foreground/20 peer-data-[active=true]/menu-button:text-sidebar-primary-foreground"
                data-testid="sidebar-home-count"
              >
                {Math.min(homeBadgeCount, 99)}
              </SidebarMenuBadge>
            ) : null}
          </SidebarMenuItem>
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
            {shouldShowAgentCount ? (
              <SidebarMenuBadge
                className="right-2 rounded-full bg-sidebar-accent/70 px-1.5 text-[11px] text-sidebar-foreground/75 peer-data-[active=true]/menu-button:bg-sidebar-primary-foreground/20 peer-data-[active=true]/menu-button:text-sidebar-primary-foreground"
                data-testid="sidebar-agents-count"
              >
                {totalAgentCount}
              </SidebarMenuBadge>
            ) : null}
          </SidebarMenuItem>
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
        </SidebarMenu>
      </SidebarHeader>

      <div className="relative flex min-h-0 flex-1 flex-col overflow-hidden">
        {unreadAboveCount > 0 ? (
          <MoreUnreadButton
            count={unreadAboveCount}
            icon={<ArrowUp />}
            onClick={scrollToNextAbove}
            position="top"
            testId="sidebar-more-unread-above"
          />
        ) : null}
        <SidebarContent className="pb-32" ref={scrollRef}>
          {isLoading ? (
            <SidebarGroup>
              <SidebarGroupLabel>Channels</SidebarGroupLabel>
              <SidebarGroupContent>
                <SidebarMenu data-testid="sidebar-loading">
                  {skeletonRows.map((row) => (
                    <SidebarMenuSkeleton key={row} showIcon />
                  ))}
                </SidebarMenu>
              </SidebarGroupContent>
            </SidebarGroup>
          ) : null}

          {!isLoading ? (
            <>
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
                  />
                ))}
                <ChannelGroupSection
                  browseAriaLabel="Browse channels"
                  browseTestId="browse-channels"
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
                  onBrowse={onOpenBrowseChannels}
                  onCreateClick={() => setCreateDialogKind("stream")}
                  onMarkAllRead={onMarkAllChannelsRead}
                  onMarkChannelRead={onMarkChannelRead}
                  onMarkChannelUnread={onMarkChannelUnread}
                  onSelectChannel={onSelectChannel}
                  onToggleCollapsed={() => toggleCollapsedGroup("channels")}
                  selectedChannelId={selectedChannelId}
                  title="Channels"
                  unreadChannelIds={unreadChannelIds}
                  sections={channelSections}
                  assignments={channelAssignments}
                  onAssignChannel={assignChannel}
                  onUnassignChannel={unassignChannel}
                  onCreateSectionForChannel={handleCreateSectionForChannel}
                />
              </SidebarDndContext>
              <ChannelGroupSection
                browseAriaLabel="Browse forums"
                browseTestId="browse-forums"
                createAriaLabel="Create a forum"
                hasUnread={unreadChannelIds.size > 0}
                isCollapsed={collapsedGroups.forums}
                isActiveChannel={selectedView === "channel"}
                items={forumChannels}
                listTestId="forum-list"
                onBrowse={onOpenBrowseForums}
                onCreateClick={() => setCreateDialogKind("forum")}
                onMarkAllRead={onMarkAllChannelsRead}
                onMarkChannelRead={onMarkChannelRead}
                onMarkChannelUnread={onMarkChannelUnread}
                onSelectChannel={onSelectChannel}
                onToggleCollapsed={() => toggleCollapsedGroup("forums")}
                selectedChannelId={selectedChannelId}
                title="Forums"
                unreadChannelIds={unreadChannelIds}
              />
              <SidebarSection
                action={
                  <SidebarGroupAction
                    aria-expanded={isNewDmOpen}
                    aria-label="Start a direct message"
                    className={cn(
                      "top-1/2 -translate-y-1/2 text-sidebar-foreground/50 hover:bg-sidebar-accent/60 hover:text-sidebar-foreground",
                      SECTION_ACTION_VISIBILITY_CLASS,
                    )}
                    data-testid="new-dm-trigger"
                    onClick={() => {
                      setIsNewDmOpen(true);
                    }}
                    type="button"
                  >
                    <PenSquare className="transition-transform" />
                  </SidebarGroupAction>
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
                unreadChannelIds={unreadChannelIds}
              />
            </>
          ) : null}

          {errorMessage ? (
            <div className="px-3 py-2 text-sm text-destructive">
              {errorMessage}
            </div>
          ) : null}
        </SidebarContent>

        {unreadBelowCount > 0 ? (
          <MoreUnreadButton
            bottomClassName="bottom-28"
            count={unreadBelowCount}
            icon={<ArrowDown />}
            onClick={scrollToNextBelow}
            position="bottom"
            testId="sidebar-more-unread-below"
          />
        ) : null}

        <SidebarFooter className="absolute inset-x-0 bottom-0 z-30 bg-sidebar/55 backdrop-blur-xl supports-[backdrop-filter]:bg-sidebar/45 dark:bg-sidebar/45 dark:supports-[backdrop-filter]:bg-sidebar/35">
          <SidebarMenu>
            <SidebarMenuItem>
              <div
                className="rounded-xl px-2 py-2 transition-colors hover:bg-sidebar-accent/35 focus-within:bg-sidebar-accent/35 dark:hover:bg-sidebar-accent/25 dark:focus-within:bg-sidebar-accent/25"
                data-testid="sidebar-profile-card"
              >
                <div className="flex min-w-0 items-center gap-3">
                  <div className="relative shrink-0">
                    <ProfileAvatar
                      avatarUrl={profile?.avatarUrl ?? null}
                      className="h-10 w-10 rounded-2xl text-sm"
                      iconClassName="h-5 w-5"
                      label={resolvedDisplayName}
                      testId="sidebar-profile-avatar"
                    />
                    <span
                      aria-label={getPresenceLabel(selfPresenceStatus)}
                      className="absolute -bottom-0.5 -right-0.5 flex h-4 w-4 items-center justify-center rounded-full bg-sidebar"
                      data-testid="self-presence-badge"
                      role="img"
                    >
                      <PresenceDot
                        className="h-2.5 w-2.5"
                        status={selfPresenceStatus}
                      />
                    </span>
                  </div>
                  <div className="min-w-0 flex-1">
                    <ProfilePopover
                      open={profilePopoverOpen}
                      onOpenChange={setProfilePopoverOpen}
                      displayName={resolvedDisplayName}
                      nip05={profile?.nip05Handle}
                      avatarUrl={profile?.avatarUrl ?? null}
                      currentStatus={selfPresenceStatus}
                      isStatusPending={isPresencePending}
                      userStatusText={selfUserStatus?.text}
                      userStatusEmoji={selfUserStatus?.emoji}
                      onSetStatus={onSetPresenceStatus ?? (() => {})}
                      onSetUserStatus={onSetUserStatus}
                      onClearUserStatus={onClearUserStatus}
                      onOpenSettings={onSelectSettings}
                    >
                      <button
                        className="block w-full min-w-0 text-left text-sidebar-foreground"
                        data-testid="open-settings"
                        type="button"
                      >
                        <p
                          className="truncate text-sm font-semibold text-current"
                          data-testid="sidebar-profile-name"
                        >
                          {resolvedDisplayName}
                        </p>
                      </button>
                    </ProfilePopover>
                    <WorkspaceSwitcher
                      activeWorkspace={activeWorkspace}
                      onAddWorkspace={onOpenAddWorkspace}
                      onRemoveWorkspace={onRemoveWorkspace}
                      onSwitchWorkspace={onSwitchWorkspace}
                      onUpdateWorkspace={onUpdateWorkspace}
                      variant="profile"
                      workspaces={workspaces}
                    />
                    {selfUserStatus?.text || selfUserStatus?.emoji ? (
                      <p className="mt-0.5 truncate text-xs text-sidebar-foreground/50">
                        {selfUserStatus.emoji ? (
                          <StatusEmoji
                            className="mr-1 h-3.5 w-3.5"
                            value={selfUserStatus.emoji}
                          />
                        ) : null}
                        {selfUserStatus.text}
                      </p>
                    ) : null}
                  </div>
                </div>
              </div>
            </SidebarMenuItem>
          </SidebarMenu>
        </SidebarFooter>
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
    </Sidebar>
  );
}
