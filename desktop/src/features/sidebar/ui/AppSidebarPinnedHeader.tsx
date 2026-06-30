import { openUrl } from "@tauri-apps/plugin-opener";
import {
  Activity,
  Bot,
  CalendarDays,
  FolderGit2,
  Inbox,
  Video,
  Zap,
} from "lucide-react";

import { TopbarSearch } from "@/features/search/ui/TopbarSearch";
import { FeatureGate } from "@/shared/features";
import type { GoogleCalendarEvent } from "@/features/calendar/api";
import {
  eventStartDate,
  isOngoingCalendarEvent,
} from "@/features/calendar/hooks";
import type { Channel, SearchHit } from "@/shared/api/types";
import {
  SidebarHeader,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/shared/ui/sidebar";

type SidebarSelectedView =
  | "home"
  | "channel"
  | "agents"
  | "workflows"
  | "pulse"
  | "projects";

function formatCalendarStart(event: GoogleCalendarEvent, nowMs: number) {
  const startsAt = eventStartDate(event).getTime();
  const minutes = Math.max(0, Math.ceil((startsAt - nowMs) / 60_000));

  if (minutes <= 0) return "Now";
  if (minutes < 60) return `in ${minutes}m`;

  const hours = Math.floor(minutes / 60);
  const remainder = minutes % 60;
  return remainder === 0 ? `in ${hours}h` : `in ${hours}h ${remainder}m`;
}

function CalendarMenuItem({
  event,
  now,
}: {
  event: GoogleCalendarEvent | null;
  now: number;
}) {
  const emptyTitle = "No upcoming meetings";
  const title = event?.title.trim() || emptyTitle;
  const isOngoing = event ? isOngoingCalendarEvent(event, now) : false;
  const joinUrl = event?.joinUrl ?? null;
  const showJoin = Boolean(isOngoing && joinUrl);
  const canOpenJoinUrl = Boolean(showJoin && joinUrl);
  const meta = event ? formatCalendarStart(event, now) : null;
  const content = (
    <>
      <CalendarDays className="h-4 w-4" />
      <span className="min-w-0 flex-1 truncate pr-2">{title}</span>
      {showJoin ? (
        <>
          <span className="shrink-0 text-2xs font-medium text-sidebar-foreground/60 group-hover/calendar:hidden group-focus-visible/calendar:hidden">
            Now
          </span>
          <span className="hidden shrink-0 items-center gap-1 rounded-full bg-primary/10 px-1.5 py-0.5 text-2xs font-medium text-primary group-hover/calendar:inline-flex group-focus-visible/calendar:inline-flex">
            <Video className="h-3 w-3" />
            Join
          </span>
        </>
      ) : meta ? (
        <span className="shrink-0 text-2xs font-medium text-sidebar-foreground/60">
          {meta}
        </span>
      ) : null}
    </>
  );

  if (!canOpenJoinUrl) {
    return (
      <SidebarMenuItem>
        <SidebarMenuButton asChild className="group/calendar" tooltip={title}>
          <div data-testid="sidebar-calendar-status">{content}</div>
        </SidebarMenuButton>
      </SidebarMenuItem>
    );
  }

  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        aria-label={showJoin ? `Join ${title}` : title}
        className="group/calendar"
        data-testid="sidebar-calendar-status"
        onClick={() => {
          if (!canOpenJoinUrl || !joinUrl) return;
          void openUrl(joinUrl);
        }}
        tooltip={title}
        type="button"
      >
        {content}
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}

type AppSidebarPinnedHeaderProps = {
  calendarEvent: GoogleCalendarEvent | null;
  calendarNow: number;
  channelLabels: Record<string, string>;
  currentPubkey?: string;
  homeBadgeCount: number;
  onCreateAgent: () => void;
  onCreateChannel: () => void;
  onOpenDm: (input: { pubkeys: string[] }) => Promise<void>;
  onOpenSearchResult: (hit: SearchHit) => void;
  onSelectAgents: () => void;
  onSelectChannel: (channelId: string) => void;
  onSelectHome: () => void;
  onSelectProjects: () => void;
  onSelectPulse: () => void;
  onSelectWorkflows: () => void;
  searchChannels: Channel[];
  searchFocusRequest: number;
  selectedView: SidebarSelectedView;
  suggestionChannels: Channel[];
};

export function AppSidebarPinnedHeader({
  calendarEvent,
  calendarNow,
  channelLabels,
  currentPubkey,
  homeBadgeCount,
  onCreateAgent,
  onCreateChannel,
  onOpenDm,
  onOpenSearchResult,
  onSelectAgents,
  onSelectChannel,
  onSelectHome,
  onSelectProjects,
  onSelectPulse,
  onSelectWorkflows,
  searchChannels,
  searchFocusRequest,
  selectedView,
  suggestionChannels,
}: AppSidebarPinnedHeaderProps) {
  return (
    <div className="shrink-0 px-2 pt-2.5" data-testid="sidebar-pinned-header">
      <TopbarSearch
        channelLabels={channelLabels}
        channels={searchChannels}
        currentPubkey={currentPubkey}
        focusRequest={searchFocusRequest}
        onOpenChannel={onSelectChannel}
        onOpenResult={onOpenSearchResult}
        onOpenUser={(user) => onOpenDm({ pubkeys: [user.pubkey] })}
        onCreateAgent={onCreateAgent}
        onCreateChannel={onCreateChannel}
        suggestionChannels={suggestionChannels}
      />
      <SidebarHeader
        className="cursor-default select-none px-0 pb-0 pt-2.5"
        data-tauri-drag-region
      >
        <SidebarMenu className="pb-1">
          <SidebarMenuItem>
            <SidebarMenuButton
              isActive={selectedView === "home"}
              onClick={onSelectHome}
              tooltip="Inbox"
              type="button"
            >
              <Inbox className="h-4 w-4" />
              <span>Inbox</span>
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
          <CalendarMenuItem event={calendarEvent} now={calendarNow} />
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
    </div>
  );
}
