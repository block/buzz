import { ChevronDown } from "lucide-react";
import { isMacPlatform } from "@/shared/lib/platform";
import { useIsFullscreen } from "@/shared/lib/useIsFullscreen";
import { TopbarSearch } from "@/features/search/ui/TopbarSearch";
import type { Channel, SearchHit } from "@/shared/api/types";
import type { Community } from "@/features/communities/types";
import type { LeaveCommunityResult } from "@/features/communities/leaveCommunity";
import {
  SidebarHeader,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/shared/ui/sidebar";
import { SidebarMenuLabel } from "@/shared/ui/sidebar-menu-label";
import { useSidebar } from "@/shared/ui/sidebar";
import {
  ActivityIcon,
  CollapseSidebarIcon,
  ExpandSidebarIcon,
  TodayIcon,
} from "@/shared/ui/colonyIcons";
import { WorkspaceMark } from "@/features/communities/ui/WorkspaceMark";
import { WorkspaceSwitcherMenu } from "@/features/communities/ui/WorkspaceSwitcherMenu";

type SidebarSelectedView =
  | "today"
  | "home"
  | "channel"
  | "messages"
  | "agents"
  | "clients"
  | "work"
  | "business"
  | "team"
  | "pins";

type AppSidebarPinnedHeaderProps = {
  activeWorkspace: Community | null;
  workspaces: Community[];
  channelLabels: Record<string, string>;
  currentChannelId?: string | null;
  currentPubkey?: string;
  onBrowseChannels?: () => void;
  onCopyWorkspaceLink: () => void;
  onCreateWorkspace: () => void;
  onCreateAgent: () => void;
  onCreateChannel: () => void;
  onOpenDm: (input: { pubkeys: string[] }) => Promise<void>;
  onOpenSearchResult: (hit: SearchHit, query: string) => void;
  onInvitePeople: () => void;
  onJoinWorkspace: () => void;
  onOpenWorkspaceSettings: () => void;
  onRemoveWorkspace: (id: string) => Promise<LeaveCommunityResult | undefined>;
  onSelectChannel: (channelId: string) => void;
  onSwitchWorkspace: (id: string) => void;
  searchChannels: Channel[];
  searchFocusRequest: number;
  showSidebarCollapseButton: boolean;
  scopeSearchFocusRequest: number;
  suggestionChannels: Channel[];
};

type AppSidebarPrimaryMenuProps = {
  homeBadgeCount: number;
  onSelectToday: () => void;
  onSelectHome: () => void;
  suppressActivitySelection?: boolean;
  suppressTodaySelection?: boolean;
  selectedView: SidebarSelectedView;
};

export function AppSidebarPinnedHeader({
  activeWorkspace,
  workspaces,
  channelLabels,
  currentChannelId,
  currentPubkey,
  onBrowseChannels,
  onCopyWorkspaceLink,
  onCreateWorkspace,
  onCreateAgent,
  onCreateChannel,
  onOpenDm,
  onOpenSearchResult,
  onInvitePeople,
  onJoinWorkspace,
  onOpenWorkspaceSettings,
  onRemoveWorkspace,
  onSelectChannel,
  onSwitchWorkspace,
  searchChannels,
  searchFocusRequest,
  showSidebarCollapseButton,
  scopeSearchFocusRequest,
  suggestionChannels,
}: AppSidebarPinnedHeaderProps) {
  const sidebar = useSidebar();
  const isFullscreen = useIsFullscreen();
  const activeWorkspaceName = activeWorkspace?.name ?? "No workspace";
  const communityInitial =
    activeWorkspaceName.trim().slice(0, 1).toUpperCase() || "C";

  return (
    <div
      className="shrink-0"
      data-testid="sidebar-pinned-header"
      data-mac-chrome={isMacPlatform() && !isFullscreen ? "true" : undefined}
    >
      <div className="colony-sidebar-brand">
        <WorkspaceSwitcherMenu
          activeWorkspace={activeWorkspace}
          workspaces={workspaces}
          onCopyWorkspaceLink={onCopyWorkspaceLink}
          onCreateWorkspace={onCreateWorkspace}
          onInvitePeople={onInvitePeople}
          onJoinWorkspace={onJoinWorkspace}
          onOpenWorkspaceSettings={onOpenWorkspaceSettings}
          onRemoveWorkspace={onRemoveWorkspace}
          onSwitchWorkspace={onSwitchWorkspace}
          renderWorkspaceMark={(workspace) => (
            <WorkspaceMark
              className="colony-sidebar-brand-mark grid h-9 w-9 shrink-0 place-items-center overflow-hidden rounded-xl bg-[var(--colony-sidebar-ink)] text-[var(--colony-accent-foreground)] text-sm font-extrabold leading-none"
              communityIconFit="cover"
              communityId={workspace.id}
              dataTestId={`workspace-menu-mark-${workspace.id}`}
              fallbackContent={
                <span data-testid="sidebar-community-brand-fallback">
                  {workspace.name.trim().slice(0, 1).toUpperCase() || "C"}
                </span>
              }
              imageFit="contain"
              name={workspace.name}
              relayUrl={workspace.relayUrl}
            />
          )}
          renderTrigger={({ mobile, open }) => (
            <button
              aria-expanded={open}
              aria-haspopup={mobile ? "dialog" : "menu"}
              aria-label={`Open workspace menu, current workspace ${activeWorkspaceName}`}
              className="min-w-0"
              data-testid="sidebar-business-switcher"
              type="button"
            >
              <WorkspaceMark
                className="colony-sidebar-brand-mark"
                communityIconFit="cover"
                communityId={activeWorkspace?.id}
                dataTestId="sidebar-community-brand-mark"
                dot={<span className="colony-sidebar-brand-dot" />}
                fallbackContent={
                  <span data-testid="sidebar-community-brand-fallback">
                    {communityInitial}
                  </span>
                }
                imageFit="contain"
                name={activeWorkspaceName}
                relayUrl={activeWorkspace?.relayUrl}
              />
              <span className="sidebar-company-name">
                <span>{activeWorkspaceName}</span>
                <span className="sidebar-company-context">
                  Company workspace
                </span>
              </span>
              <ChevronDown aria-hidden="true" className="h-3.5 w-3.5" />
            </button>
          )}
        />
        {showSidebarCollapseButton ? (
          <button
            aria-expanded={sidebar.open}
            aria-label="Toggle Sidebar"
            className="colony-sidebar-collapse"
            data-sidebar="trigger"
            onClick={sidebar.toggleSidebar}
            type="button"
          >
            {sidebar.open ? (
              <CollapseSidebarIcon size={18} />
            ) : (
              <ExpandSidebarIcon size={18} />
            )}
          </button>
        ) : null}
      </div>
      <TopbarSearch
        channelLabels={channelLabels}
        channels={searchChannels}
        currentChannelId={currentChannelId}
        currentPubkey={currentPubkey}
        focusRequest={searchFocusRequest}
        onOpenChannel={onSelectChannel}
        onOpenResult={onOpenSearchResult}
        onOpenUser={(user) => onOpenDm({ pubkeys: [user.pubkey] })}
        onBrowseChannels={onBrowseChannels}
        onCreateAgent={onCreateAgent}
        onCreateChannel={onCreateChannel}
        scopeFocusRequest={scopeSearchFocusRequest}
        suggestionChannels={suggestionChannels}
      />
    </div>
  );
}

export function AppSidebarPrimaryMenu({
  homeBadgeCount,
  onSelectToday,
  onSelectHome,
  suppressActivitySelection = false,
  suppressTodaySelection = false,
  selectedView,
}: AppSidebarPrimaryMenuProps) {
  return (
    <SidebarHeader
      className="relative z-40 cursor-default select-none"
      data-tauri-drag-region
      data-testid="sidebar-primary-menu"
    >
      <SidebarMenu className="sidebar-primary-menu">
        <SidebarMenuItem>
          <SidebarMenuButton
            isActive={selectedView === "today" && !suppressTodaySelection}
            onClick={(event) => {
              const button = event.currentTarget;
              onSelectToday();
              window.requestAnimationFrame(() => {
                if (button.isConnected) button.focus({ preventScroll: true });
              });
            }}
            tooltip="Today"
            type="button"
          >
            <TodayIcon />
            <SidebarMenuLabel>Today</SidebarMenuLabel>
          </SidebarMenuButton>
        </SidebarMenuItem>
        <SidebarMenuItem data-testid="sidebar-activity-item">
          <SidebarMenuButton
            data-testid="sidebar-activity-button"
            isActive={selectedView === "home" && !suppressActivitySelection}
            onClick={() => {
              onSelectHome();
              window.requestAnimationFrame(() => {
                window.requestAnimationFrame(() => {
                  document
                    .querySelector<HTMLButtonElement>(
                      '[data-testid="sidebar-activity-button"]',
                    )
                    ?.focus({ preventScroll: true });
                });
              });
            }}
            tooltip="Activity"
            type="button"
          >
            <ActivityIcon />
            <SidebarMenuLabel>Activity</SidebarMenuLabel>
          </SidebarMenuButton>
          {homeBadgeCount > 0 ? (
            <SidebarMenuBadge data-testid="sidebar-home-count">
              {Math.min(homeBadgeCount, 99)}
            </SidebarMenuBadge>
          ) : null}
        </SidebarMenuItem>
      </SidebarMenu>
    </SidebarHeader>
  );
}
