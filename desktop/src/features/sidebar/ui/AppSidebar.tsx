// biome-ignore format: keep compact to stay within file size limit
import {
  Activity,
  Bot,
  ChevronDown,
  CircleDot,
  FolderGit2,
  Home,
  PenSquare,
  Plus,
  Search,
  Zap,
} from "lucide-react";
import * as React from "react";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import type { Workspace } from "@/features/workspaces/types";
import { AddWorkspaceDialog } from "@/features/workspaces/ui/AddWorkspaceDialog";
import { WorkspaceSwitcher } from "@/features/workspaces/ui/WorkspaceSwitcher";
import { useDeferredLoad } from "@/shared/hooks/useDeferredStartup";
import { getPresenceLabel } from "@/features/presence/lib/presence";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { ProfilePopover } from "@/features/profile/ui/ProfilePopover";
import { useDmSidebarMetadata } from "@/features/sidebar/useDmSidebarMetadata";
import {
  ChannelMenuButton,
  SidebarSection,
} from "@/features/sidebar/ui/SidebarSection";
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
import { Button } from "@/shared/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from "@/shared/ui/context-menu";
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

// ---------------------------------------------------------------------------
// Shared styles
// ---------------------------------------------------------------------------

const SECTION_ICON_BUTTON_CLASS =
  "flex h-5 w-5 items-center justify-center rounded-md text-sidebar-foreground/50 hover:bg-sidebar-accent/60 hover:text-sidebar-foreground";
const SECTION_ACTION_VISIBILITY_CLASS =
  "opacity-0 transition-opacity group-hover/sidebar-section:opacity-100 group-focus-within/sidebar-section:opacity-100";
const SECTION_LABEL_BUTTON_CLASS =
  "group/section-label flex w-fit max-w-[calc(100%-3rem)] cursor-pointer appearance-none items-center gap-1 text-left transition-colors hover:text-sidebar-foreground focus-visible:text-sidebar-foreground";
const SECTION_LABEL_CHEVRON_CLASS =
  "h-2.5 w-2.5 shrink-0 opacity-0 text-sidebar-foreground/45 transition-[color,opacity,transform] group-hover/section-label:opacity-100 group-hover/section-label:text-sidebar-foreground group-focus-visible/section-label:opacity-100 group-focus-visible/section-label:text-sidebar-foreground";

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
  onOpenSearch: () => void;
  onHideDm: (channelId: string) => void;
  onMarkChannelUnread: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
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
};

// ---------------------------------------------------------------------------
// SectionHeaderActions — browse + create icon buttons for section headers
// ---------------------------------------------------------------------------

function SectionHeaderActions({
  browseAriaLabel,
  browseTestId,
  className,
  createAriaLabel,
  onBrowse,
  onCreateClick,
}: {
  browseAriaLabel: string;
  browseTestId?: string;
  className?: string;
  createAriaLabel: string;
  onBrowse: () => void;
  onCreateClick: () => void;
}) {
  return (
    <div
      className={cn(
        "absolute right-1 top-1/2 z-10 flex -translate-y-1/2 items-center gap-0.5",
        className,
      )}
    >
      <button
        aria-label={browseAriaLabel}
        className={SECTION_ICON_BUTTON_CLASS}
        data-testid={browseTestId}
        onClick={onBrowse}
        type="button"
      >
        <Search className="h-3.5 w-3.5" />
      </button>
      <button
        aria-label={createAriaLabel}
        className={SECTION_ICON_BUTTON_CLASS}
        onClick={onCreateClick}
        type="button"
      >
        <Plus className="h-4 w-4" />
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// ChannelGroupSection — unified Channels / Forums section (no inline form)
// ---------------------------------------------------------------------------

function ChannelGroupSection({
  browseAriaLabel,
  browseTestId,
  createAriaLabel,
  groupClassName,
  isCollapsed,
  isActiveChannel,
  items,
  listTestId,
  onBrowse,
  onCreateClick,
  onMarkChannelUnread,
  onSelectChannel,
  onToggleCollapsed,
  selectedChannelId,
  title,
  unreadChannelIds,
}: {
  browseAriaLabel: string;
  browseTestId?: string;
  createAriaLabel: string;
  groupClassName?: string;
  isCollapsed: boolean;
  isActiveChannel: boolean;
  items: Channel[];
  listTestId: string;
  onBrowse: () => void;
  onCreateClick: () => void;
  onMarkChannelUnread: (
    channelId: string,
    lastMessageAt: string | null | undefined,
  ) => void;
  onSelectChannel: (channelId: string) => void;
  onToggleCollapsed: () => void;
  selectedChannelId: string | null;
  title: string;
  unreadChannelIds: Set<string>;
}) {
  const contentId = `sidebar-${listTestId}`;

  return (
    <SidebarGroup className={groupClassName}>
      <div className="group/sidebar-section relative">
        <SidebarGroupLabel asChild>
          <button
            aria-controls={contentId}
            aria-expanded={!isCollapsed}
            className={SECTION_LABEL_BUTTON_CLASS}
            onClick={onToggleCollapsed}
            type="button"
          >
            <span>{title}</span>
            <ChevronDown
              aria-hidden="true"
              className={cn(
                SECTION_LABEL_CHEVRON_CLASS,
                isCollapsed ? "-rotate-90" : "rotate-0",
              )}
            />
          </button>
        </SidebarGroupLabel>
        <SectionHeaderActions
          browseAriaLabel={browseAriaLabel}
          browseTestId={browseTestId}
          className={SECTION_ACTION_VISIBILITY_CLASS}
          createAriaLabel={createAriaLabel}
          onBrowse={onBrowse}
          onCreateClick={onCreateClick}
        />
      </div>
      {!isCollapsed ? (
        <SidebarGroupContent id={contentId}>
          {items.length > 0 ? (
            <SidebarMenu data-testid={listTestId}>
              {items.map((channel) => (
                <ContextMenu key={channel.id}>
                  <ContextMenuTrigger asChild>
                    <SidebarMenuItem>
                      <ChannelMenuButton
                        channel={channel}
                        hasUnread={unreadChannelIds.has(channel.id)}
                        isActive={
                          isActiveChannel && selectedChannelId === channel.id
                        }
                        onSelectChannel={onSelectChannel}
                      />
                    </SidebarMenuItem>
                  </ContextMenuTrigger>
                  <ContextMenuContent>
                    <ContextMenuItem
                      onClick={() =>
                        onMarkChannelUnread(channel.id, channel.lastMessageAt)
                      }
                    >
                      <CircleDot className="h-4 w-4" />
                      Mark unread
                    </ContextMenuItem>
                  </ContextMenuContent>
                </ContextMenu>
              ))}
            </SidebarMenu>
          ) : null}
        </SidebarGroupContent>
      ) : null}
    </SidebarGroup>
  );
}

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
  onOpenSearch,
  onHideDm,
  onMarkChannelUnread,
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
}: AppSidebarProps) {
  const skeletonRows = ["first", "second", "third", "fourth", "fifth", "sixth"];
  const [isNewDmOpenInternal, setIsNewDmOpenInternal] = React.useState(false);
  const isNewDmOpen = isNewDmOpenProp ?? isNewDmOpenInternal;
  const setIsNewDmOpen = onNewDmOpenChange ?? setIsNewDmOpenInternal;
  const [profilePopoverOpen, setProfilePopoverOpen] = React.useState(false);
  const [createDialogKind, setCreateDialogKind] =
    React.useState<CreateChannelKind | null>(null);
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

  const streamChannels = React.useMemo(
    () => channels.filter((channel) => channel.channelType === "stream"),
    [channels],
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
        className="cursor-default select-none gap-3 pt-10"
        data-tauri-drag-region
      >
        <div className="px-0.5">
          <WorkspaceSwitcher
            activeWorkspace={activeWorkspace}
            onAddWorkspace={onOpenAddWorkspace}
            onRemoveWorkspace={onRemoveWorkspace}
            onSwitchWorkspace={onSwitchWorkspace}
            onUpdateWorkspace={onUpdateWorkspace}
            workspaces={workspaces}
          />
        </div>
        <Button
          className="w-full justify-between rounded-xl border border-sidebar-border/80 bg-sidebar-accent/60 px-3 text-sidebar-foreground/80 shadow-sm hover:bg-sidebar-accent hover:text-sidebar-foreground"
          data-testid="open-search"
          onClick={onOpenSearch}
          size="sm"
          type="button"
          variant="ghost"
        >
          <span className="flex items-center gap-2">
            <Search className="h-4 w-4" />
            Search messages
          </span>
          <span className="text-xs text-sidebar-foreground/50">&#x2318;K</span>
        </Button>
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

      <SidebarContent>
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
            <ChannelGroupSection
              browseAriaLabel="Browse channels"
              browseTestId="browse-channels"
              createAriaLabel="Create a channel"
              groupClassName="pt-1"
              isCollapsed={collapsedGroups.channels}
              isActiveChannel={selectedView === "channel"}
              items={streamChannels}
              listTestId="stream-list"
              onBrowse={onOpenBrowseChannels}
              onCreateClick={() => setCreateDialogKind("stream")}
              onMarkChannelUnread={onMarkChannelUnread}
              onSelectChannel={onSelectChannel}
              onToggleCollapsed={() => toggleCollapsedGroup("channels")}
              selectedChannelId={selectedChannelId}
              title="Channels"
              unreadChannelIds={unreadChannelIds}
            />
            <ChannelGroupSection
              browseAriaLabel="Browse forums"
              browseTestId="browse-forums"
              createAriaLabel="Create a forum"
              isCollapsed={collapsedGroups.forums}
              isActiveChannel={selectedView === "channel"}
              items={forumChannels}
              listTestId="forum-list"
              onBrowse={onOpenBrowseForums}
              onCreateClick={() => setCreateDialogKind("forum")}
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

      <SidebarFooter>
        <SidebarMenu>
          <SidebarMenuItem>
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
              <SidebarMenuButton
                className="h-auto gap-3 rounded-xl px-2 py-2"
                data-testid="open-settings"
                type="button"
              >
                <div
                  className="flex min-w-0 flex-1 items-center gap-3"
                  data-testid="sidebar-profile-card"
                >
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
                  <div className="min-w-0">
                    <p
                      className="truncate text-sm font-semibold text-current"
                      data-testid="sidebar-profile-name"
                    >
                      {resolvedDisplayName}
                    </p>
                    {selfUserStatus?.text || selfUserStatus?.emoji ? (
                      <p className="truncate text-xs text-sidebar-foreground/50">
                        {selfUserStatus.emoji ? (
                          <span className="mr-1">{selfUserStatus.emoji}</span>
                        ) : null}
                        {selfUserStatus.text}
                      </p>
                    ) : null}
                  </div>
                </div>
              </SidebarMenuButton>
            </ProfilePopover>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>

      <CreateChannelDialog
        channelKind={createDialogKind}
        isCreating={isCreatingAny}
        onOpenChange={(open) => {
          if (!open) setCreateDialogKind(null);
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
    </Sidebar>
  );
}
