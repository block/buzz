import { Activity, Bot, Folders, Inbox, Zap } from "lucide-react";
import * as React from "react";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { pickBestieAgent } from "@/features/agents/lib/bestie";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { TopbarSearch } from "@/features/search/ui/TopbarSearch";
import { SidebarProjectsSection } from "@/features/sidebar/ui/SidebarProjectsSection";
import { FeatureGate } from "@/shared/features";
import type { OpenDmInput } from "@/shared/api/tauriChannels";
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

type AppSidebarPinnedHeaderProps = {
  channelLabels: Record<string, string>;
  currentChannelId?: string | null;
  currentPubkey?: string;
  onBrowseChannels?: () => void;
  onCreateAgent: () => void;
  onCreateChannel: () => void;
  onOpenDm: (input: OpenDmInput) => Promise<void>;
  onOpenSearchResult: (hit: SearchHit, query: string) => void;
  onSelectChannel: (channelId: string) => void;
  searchChannels: Channel[];
  searchFocusRequest: number;
  scopeSearchFocusRequest: number;
  suggestionChannels: Channel[];
};

type AppSidebarPrimaryMenuProps = {
  bestieRelayUrl?: string | null;
  currentPubkey?: string;
  homeBadgeCount: number;
  onOpenDm: (input: OpenDmInput) => Promise<void>;
  onSelectAgents: () => void;
  onSelectHome: () => void;
  onSelectProjects: () => void;
  onSelectPulse: () => void;
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

export function AppSidebarPrimaryMenu({
  bestieRelayUrl,
  currentPubkey,
  homeBadgeCount,
  onOpenDm,
  onSelectAgents,
  onSelectHome,
  onSelectProjects,
  onSelectPulse,
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
          <FeatureGate feature="bestie">
            <BestieSidebarMenuItem
              currentPubkey={currentPubkey}
              onOpenDm={onOpenDm}
              relayUrl={bestieRelayUrl}
            />
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

function BestieSidebarMenuItem({
  currentPubkey,
  onOpenDm,
  relayUrl,
}: {
  currentPubkey?: string;
  onOpenDm: (input: OpenDmInput) => Promise<void>;
  relayUrl?: string | null;
}) {
  const managedAgentsQuery = useManagedAgentsQuery();
  const bestieAgent = React.useMemo(
    () => pickBestieAgent(managedAgentsQuery.data ?? [], relayUrl),
    [managedAgentsQuery.data, relayUrl],
  );

  if (!bestieAgent) return null;

  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        data-testid="open-bestie-dm"
        onClick={() => {
          const expectedRelayUrl = relayUrl?.trim();
          const expectedSignerPubkey = currentPubkey?.trim();
          if (!expectedRelayUrl || !expectedSignerPubkey) return;
          void onOpenDm({
            expectedRelayUrl,
            expectedSignerPubkey,
            pubkeys: [bestieAgent.pubkey],
          });
        }}
        tooltip={`Message ${bestieAgent.name}`}
        type="button"
      >
        <ProfileAvatar
          avatarUrl={bestieAgent.avatarUrl}
          className="size-4 text-3xs shadow-none"
          label={bestieAgent.name}
          plain
          testId="bestie-sidebar-avatar"
        />
        <SidebarMenuLabel>Bestie</SidebarMenuLabel>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}
