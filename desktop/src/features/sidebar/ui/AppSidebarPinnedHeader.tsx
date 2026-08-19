import { Activity, Bot, Folders, Inbox, Zap } from "lucide-react";

import { TopbarSearch } from "@/features/search/ui/TopbarSearch";
import { FeatureGate } from "@/shared/features";
import type { Channel, SearchHit } from "@/shared/api/types";
import {
  SidebarHeader,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/shared/ui/sidebar";
import { SidebarMenuLabel } from "@/shared/ui/sidebar-menu-label";

type SidebarSelectedView =
  | "home"
  | "channel"
  | "messages"
  | "agents"
  | "workflows"
  | "pulse"
  | "projects";

type AppSidebarPrimaryMenuProps = {
  homeBadgeCount: number;
  onSelectAgents: () => void;
  onSelectHome: () => void;
  onSelectProjects: () => void;
  onSelectPulse: () => void;
  onSelectWorkflows: () => void;
  selectedView: SidebarSelectedView;
};

type AppSidebarPinnedHeaderProps = AppSidebarPrimaryMenuProps & {
  channelLabels: Record<string, string>;
  currentChannelId?: string | null;
  currentPubkey?: string;
  onBrowseChannels?: () => void;
  onCreateAgent: () => void;
  onCreateChannel: () => void;
  onOpenDm: (input: { pubkeys: string[] }) => Promise<void>;
  onOpenSearchResult: (hit: SearchHit) => void;
  onSelectChannel: (channelId: string) => void;
  searchChannels: Channel[];
  searchFocusRequest: number;
  scopeSearchFocusRequest: number;
  suggestionChannels: Channel[];
};

export function AppSidebarPinnedHeader({
  channelLabels,
  currentChannelId,
  currentPubkey,
  homeBadgeCount,
  onBrowseChannels,
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
  scopeSearchFocusRequest,
  selectedView,
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
        className="mb-2"
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
      <AppSidebarPrimaryMenu
        homeBadgeCount={homeBadgeCount}
        onSelectAgents={onSelectAgents}
        onSelectHome={onSelectHome}
        onSelectProjects={onSelectProjects}
        onSelectPulse={onSelectPulse}
        onSelectWorkflows={onSelectWorkflows}
        selectedView={selectedView}
      />
    </div>
  );
}

export function AppSidebarPrimaryMenu({
  homeBadgeCount,
  onSelectAgents,
  onSelectHome,
  onSelectProjects,
  onSelectPulse,
  onSelectWorkflows,
  selectedView,
}: AppSidebarPrimaryMenuProps) {
  return (
    <SidebarHeader
      className="relative z-40 cursor-default select-none px-0 pb-2 pt-0"
      data-tauri-drag-region
      data-testid="sidebar-primary-menu"
    >
      <SidebarMenu>
        <SidebarMenuItem>
          <SidebarMenuButton
            className="data-[active=true]:font-normal"
            isActive={selectedView === "home"}
            onClick={onSelectHome}
            tooltip="Inbox"
            type="button"
          >
            <Inbox
              className={
                selectedView !== "home" ? "h-4 w-4 opacity-80" : "h-4 w-4"
              }
            />
            <SidebarMenuLabel
              className={selectedView !== "home" ? "opacity-80" : undefined}
            >
              Inbox
            </SidebarMenuLabel>
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
              <SidebarMenuLabel>Pulse</SidebarMenuLabel>
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
            <Bot
              className={
                selectedView !== "agents" ? "h-4 w-4 opacity-80" : "h-4 w-4"
              }
            />
            <SidebarMenuLabel
              className={selectedView !== "agents" ? "opacity-80" : undefined}
            >
              Agents
            </SidebarMenuLabel>
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
              <SidebarMenuLabel>Workflows</SidebarMenuLabel>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </FeatureGate>
      </SidebarMenu>
    </SidebarHeader>
  );
}
