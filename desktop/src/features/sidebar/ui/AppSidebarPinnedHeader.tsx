import {
  Activity,
  Bot,
  BotMessageSquare,
  Folders,
  Inbox,
  Zap,
} from "lucide-react";

import type { AppView } from "@/app/AppShell.helpers";
import { PlaygroundSection } from "@/features/playground/ui/PlaygroundSection";
import { WindowsSection } from "@/features/popout/ui/WindowsSection";
import {
  parkPlaygroundHost,
  parkPlaygroundThen,
} from "@/features/playground/lib/sessions";
import { usePinnedSites } from "@/features/pinned-sites/hooks";
import { getPinnedSiteIcon } from "@/features/pinned-sites/lib/icons";
import type { PinnedSite } from "@/features/pinned-sites/lib/types";
import { TopbarSearch } from "@/features/search/ui/TopbarSearch";
import { SidebarProjectsSection } from "@/features/sidebar/ui/SidebarProjectsSection";
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
import { ProtectedBestieSidebarEntry } from "@protected-feature-components";

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
  onSelectBots: () => void;
  onSelectHome: () => void;
  onSelectPinnedSite: (pinId: string) => void;
  onSelectProjects: () => void;
  onSelectPulse: () => void;
  onSelectWorkflows: () => void;
  projectsOverviewActive: boolean;
  selectedPinId: string | null;
  selectedView: AppView;
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
        onOpenChannel={(channelId) => {
          parkPlaygroundHost();
          onSelectChannel(channelId);
        }}
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
  onSelectAgents,
  onSelectBots,
  onSelectHome,
  onSelectPinnedSite,
  onSelectProjects,
  onSelectPulse,
  onSelectWorkflows,
  projectsOverviewActive,
  selectedPinId,
  selectedView,
}: AppSidebarPrimaryMenuProps) {
  const { pins } = usePinnedSites();
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
              onClick={parkPlaygroundThen(onSelectHome)}
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
                onClick={parkPlaygroundThen(onSelectPulse)}
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
                onClick={parkPlaygroundThen(onSelectProjects)}
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
              onClick={parkPlaygroundThen(onSelectAgents)}
              tooltip="Agents"
              type="button"
            >
              <Bot className="h-4 w-4" />
              <SidebarMenuLabel>Agents</SidebarMenuLabel>
            </SidebarMenuButton>
          </SidebarMenuItem>
          <SidebarMenuItem>
            <SidebarMenuButton
              className="data-[active=true]:font-normal"
              data-testid="open-bots-view"
              isActive={selectedView === "bots"}
              onClick={parkPlaygroundThen(onSelectBots)}
              tooltip="Bots"
              type="button"
            >
              <BotMessageSquare className="h-4 w-4" />
              <SidebarMenuLabel>Bots</SidebarMenuLabel>
            </SidebarMenuButton>
          </SidebarMenuItem>
          <ProtectedBestieSidebarEntry />
          <FeatureGate feature="workflows">
            <SidebarMenuItem>
              <SidebarMenuButton
                data-testid="open-workflows-view"
                isActive={selectedView === "workflows"}
                onClick={parkPlaygroundThen(onSelectWorkflows)}
                tooltip="Workflows"
                type="button"
              >
                <Zap className="h-4 w-4" />
                <SidebarMenuLabel>Workflows</SidebarMenuLabel>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </FeatureGate>
          {pins.map((pin) => (
            <PinnedSiteMenuItem
              isActive={selectedView === "pin" && selectedPinId === pin.id}
              key={pin.id}
              onSelect={parkPlaygroundThen(() => onSelectPinnedSite(pin.id))}
              pin={pin}
            />
          ))}
        </SidebarMenu>
        <PlaygroundSection />
        <WindowsSection />
      </SidebarHeader>
      <SidebarProjectsSection />
    </>
  );
}

function PinnedSiteMenuItem({
  isActive,
  onSelect,
  pin,
}: {
  isActive: boolean;
  onSelect: () => void;
  pin: PinnedSite;
}) {
  const Icon = getPinnedSiteIcon(pin.icon);
  return (
    <SidebarMenuItem>
      <SidebarMenuButton
        className="data-[active=true]:font-normal"
        data-testid={`open-pinned-site-${pin.id}`}
        isActive={isActive}
        onClick={onSelect}
        tooltip={pin.name}
        type="button"
      >
        <Icon className="h-4 w-4" />
        <SidebarMenuLabel>{pin.name}</SidebarMenuLabel>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}
