import {
  Activity,
  Bot,
  FolderGit2,
  Inbox,
  MessageCircle,
  Zap,
} from "lucide-react";

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

type SidebarSelectedView =
  | "home"
  | "channel"
  | "chats"
  | "agents"
  | "workflows"
  | "pulse"
  | "projects";

type AppSidebarPinnedHeaderProps = {
  channelLabels: Record<string, string>;
  currentPubkey?: string;
  homeBadgeCount: number;
  onCreateAgent: () => void;
  onCreateChannel: () => void;
  onOpenDm: (input: { pubkeys: string[] }) => Promise<void>;
  onOpenSearchResult: (hit: SearchHit) => void;
  onSelectAgents: () => void;
  onSelectChats: () => void;
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
  channelLabels,
  currentPubkey,
  homeBadgeCount,
  onCreateAgent,
  onCreateChannel,
  onOpenDm,
  onOpenSearchResult,
  onSelectAgents,
  onSelectChats,
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
    <div className="shrink-0 px-2" data-testid="sidebar-pinned-header">
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
        <SidebarMenu className="pb-2">
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
              data-testid="open-chats-view"
              isActive={selectedView === "chats"}
              onClick={onSelectChats}
              tooltip="Chats"
              type="button"
            >
              <MessageCircle className="h-4 w-4" />
              <span>Chats</span>
            </SidebarMenuButton>
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
    </div>
  );
}
