import * as React from "react";
import {
  Activity,
  Bot,
  Folders,
  Inbox,
  MessagesSquare,
  Zap,
} from "lucide-react";

import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import { TopbarSearch } from "@/features/search/ui/TopbarSearch";
import { SidebarProjectsSection } from "@/features/sidebar/ui/SidebarProjectsSection";
import { useChannelsQuery } from "@/features/channels/hooks";
import {
  useThreadTitleLiveSync,
  useTitledThreadUnreadCounts,
  useWorkingTitledThreads,
} from "@/features/threads/hooks";
import type { WorkingTitledThreads } from "@/features/threads/lib/threadTitles";
import { FeatureGate } from "@/shared/features";
import { cn } from "@/shared/lib/cn";
import { useNow } from "@/shared/lib/useNow";
import type { Channel, SearchHit } from "@/shared/api/types";
import {
  SidebarHeader,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/shared/ui/sidebar";
import { SidebarMenuLabel } from "@/shared/ui/sidebar-menu-label";
import { ProtectedBestieSidebarEntry } from "@protected-feature-components";

type SidebarSelectedView =
  | "home"
  | "channel"
  | "messages"
  | "agents"
  | "workflows"
  | "pulse"
  | "projects"
  | "threads";

type AppSidebarPinnedHeaderProps = {
  channelLabels: Record<string, string>;
  currentChannelId?: string | null;
  currentPubkey?: string;
  onBrowseChannels?: () => void;
  onCreateAgent: () => void;
  onCreateChannel: () => void;
  onOpenDm: (input: { pubkeys: string[] }) => Promise<void>;
  onOpenSearchResult: (hit: SearchHit, query: string) => void;
  onSelectChannel: (channelId: string) => void;
  searchChannels: Channel[];
  searchFocusRequest: number;
  scopeSearchFocusRequest: number;
  suggestionChannels: Channel[];
};

type AppSidebarPrimaryMenuProps = {
  homeBadgeCount: number;
  onSelectAgents: () => void;
  onSelectHome: () => void;
  onSelectProjects: () => void;
  onSelectPulse: () => void;
  onSelectThreads: () => void;
  onSelectWorkflows: () => void;
  projectsOverviewActive: boolean;
  selectedView: SidebarSelectedView;
};

export function AppSidebarPinnedHeader({
  channelLabels,
  currentChannelId,
  currentPubkey,
  onBrowseChannels,
  onCreateAgent,
  onCreateChannel,
  onOpenDm,
  onOpenSearchResult,
  onSelectChannel,
  searchChannels,
  searchFocusRequest,
  scopeSearchFocusRequest,
  suggestionChannels,
}: AppSidebarPinnedHeaderProps) {
  return (
    <div
      className="mx-[3px] shrink-0 px-2 pb-2 pt-3"
      data-testid="sidebar-pinned-header"
    >
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

/** The sidebar channel badge, rolled up across working threads. */
function ThreadsWorkingBadge({
  isActive,
  working,
}: {
  isActive: boolean;
  working: WorkingTitledThreads;
}) {
  const now = useNow(1000);
  const elapsed = formatElapsed(now - working.anchorAt);
  const { threadCount } = working;
  return (
    <span
      className={cn(
        "max-w-32 shrink-0 truncate rounded-full px-1.5 py-0.5 text-2xs font-medium leading-none tabular-nums motion-safe:animate-pulse group-data-[collapsible=icon]:hidden",
        "hidden sm:inline-flex",
        isActive
          ? "bg-sidebar-active-foreground/20 text-sidebar-active-foreground"
          : "bg-primary/10 text-primary",
      )}
      data-testid="threads-working"
      title={`${threadCount} ${threadCount === 1 ? "thread" : "threads"} working`}
    >
      {threadCount > 1 ? `${elapsed} (${threadCount})` : elapsed}
    </span>
  );
}

/**
 * Bold with a dot while any titled thread has unread replies, and a working
 * timer while an agent runs in one, like a channel row.
 */
function ThreadsMenuItem({
  isActive,
  onSelect,
}: {
  isActive: boolean;
  onSelect: () => void;
}) {
  const channelsQuery = useChannelsQuery();
  // Titles change in channels the user isn't looking at (an agent naming a
  // new thread); without this the dot waits for the next refetch.
  const joinedChannelIds = React.useMemo(
    () =>
      (channelsQuery.data ?? [])
        .filter((channel) => channel.isMember)
        .map((channel) => channel.id),
    [channelsQuery.data],
  );
  useThreadTitleLiveSync(joinedChannelIds);
  const hasUnread = useTitledThreadUnreadCounts().size > 0;
  const working = useWorkingTitledThreads();
  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        className={cn(
          "data-[active=true]:font-normal",
          hasUnread &&
            "font-bold text-sidebar-foreground hover:text-sidebar-foreground data-[active=true]:font-bold",
        )}
        data-testid="open-threads-view"
        isActive={isActive}
        onClick={onSelect}
        tooltip="Threads"
        type="button"
      >
        <MessagesSquare className="h-4 w-4" />
        {/* Grows like a channel row's label, so the badge and dot sit right. */}
        <SidebarMenuLabel className="flex-1">Threads</SidebarMenuLabel>
        {working ? (
          <ThreadsWorkingBadge isActive={isActive} working={working} />
        ) : null}
        {hasUnread ? (
          <span
            // On the selected row's primary fill a primary dot vanishes, and
            // that is exactly when a thread pane covers the list.
            className={cn(
              "ml-auto h-2 w-2 shrink-0 rounded-full",
              isActive ? "bg-sidebar-active-foreground" : "bg-primary",
            )}
            data-testid="threads-unread-dot"
          >
            <span className="sr-only">unread</span>
          </span>
        ) : null}
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}

export function AppSidebarPrimaryMenu({
  homeBadgeCount,
  onSelectAgents,
  onSelectHome,
  onSelectProjects,
  onSelectPulse,
  onSelectThreads,
  onSelectWorkflows,
  projectsOverviewActive,
  selectedView,
}: AppSidebarPrimaryMenuProps) {
  return (
    <>
      <SidebarHeader
        className="relative z-40 cursor-default select-none px-2 pb-0 pt-0"
        data-tauri-drag-region
        data-testid="sidebar-primary-menu"
      >
        <SidebarMenu className="sidebar-primary-menu pb-2">
          <SidebarMenuItem>
            <SidebarMenuButton
              className="data-[active=true]:font-normal"
              isActive={selectedView === "home"}
              onClick={onSelectHome}
              tooltip="Inbox"
              type="button"
            >
              <Inbox className="h-4 w-4" />
              <SidebarMenuLabel>Inbox</SidebarMenuLabel>
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
          <ThreadsMenuItem
            isActive={selectedView === "threads"}
            onSelect={onSelectThreads}
          />
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
                <SidebarMenuLabel>Pulse</SidebarMenuLabel>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </FeatureGate>
          <FeatureGate feature="projects">
            <SidebarMenuItem>
              <SidebarMenuButton
                data-testid="open-projects-view"
                isActive={selectedView === "projects" && projectsOverviewActive}
                onClick={onSelectProjects}
                tooltip="Projects"
                type="button"
              >
                <Folders className="h-4 w-4" />
                <SidebarMenuLabel>Projects</SidebarMenuLabel>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </FeatureGate>
          <SidebarMenuItem>
            <SidebarMenuButton
              className="data-[active=true]:font-normal"
              data-testid="open-agents-view"
              isActive={selectedView === "agents"}
              onClick={onSelectAgents}
              tooltip="Agents"
              type="button"
            >
              <Bot className="h-4 w-4" />
              <SidebarMenuLabel>Agents</SidebarMenuLabel>
            </SidebarMenuButton>
          </SidebarMenuItem>
          <ProtectedBestieSidebarEntry />
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
                <SidebarMenuLabel>Workflows</SidebarMenuLabel>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </FeatureGate>
        </SidebarMenu>
      </SidebarHeader>
      <SidebarProjectsSection />
    </>
  );
}
