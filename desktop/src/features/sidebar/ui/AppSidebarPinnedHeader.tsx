import {
  Activity,
  AppWindow,
  Bot,
  BotMessageSquare,
  Folders,
  Inbox,
  SquareTerminal,
  Zap,
} from "lucide-react";

import type { AppView } from "@/app/AppShell.helpers";
import {
  parkPlaygroundHost,
  parkPlaygroundThen,
} from "@/features/playground/lib/sessions";
import {
  isLeftNavBuzzTermActive,
  isPrimaryNavRowActive,
  leaveLeftNavBuzzTerm,
  leaveLeftNavBuzzTermThen,
  openTerminalPanel,
  useTerminalPanel,
} from "@/features/terminal/terminalPanelStore";
import { usePinnedSites } from "@/features/pinned-sites/hooks";
import { getPinnedSiteIcon } from "@/features/pinned-sites/lib/icons";
import type { PinnedSite } from "@/features/pinned-sites/lib/types";
import { TopbarSearch } from "@/features/search/ui/TopbarSearch";
import { useSidebarMenuCounts } from "@/features/sidebar/lib/useSidebarMenuCounts";
import { SidebarMenuCountBadge } from "@/features/sidebar/ui/SidebarMenuCountBadge";
import { SidebarProjectsSection } from "@/features/sidebar/ui/SidebarProjectsSection";
import type { Channel, SearchHit } from "@/shared/api/types";
import { FeatureGate } from "@/shared/features";
import {
  SidebarHeader,
  SidebarMenu,
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
  onSelectAgents: () => void;
  onSelectBrowsers: () => void;
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
          leaveLeftNavBuzzTerm();
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
  onSelectAgents,
  onSelectBrowsers,
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
  const { preferences, counts } = useSidebarMenuCounts();
  const terminalPanel = useTerminalPanel();
  const buzzTermActive = isLeftNavBuzzTermActive(terminalPanel);
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
              isActive={isPrimaryNavRowActive(selectedView === "home", terminalPanel)}
              onClick={parkPlaygroundThen(leaveLeftNavBuzzTermThen(onSelectHome))}
              tooltip="Inbox"
              type="button"
            >
              <Inbox className="h-4 w-4" />
              <SidebarMenuLabel>Inbox</SidebarMenuLabel>
            </SidebarMenuButton>
            <SidebarMenuCountBadge
              count={counts.inbox}
              legacyWhenPositive
              preferenceEnabled={preferences.inbox}
              testId="sidebar-home-count"
            />
          </SidebarMenuItem>
          <FeatureGate feature="pulse">
            <SidebarMenuItem>
              <SidebarMenuButton
                data-testid="open-pulse-view"
                isActive={isPrimaryNavRowActive(selectedView === "pulse", terminalPanel)}
                onClick={parkPlaygroundThen(leaveLeftNavBuzzTermThen(onSelectPulse))}
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
                isActive={isPrimaryNavRowActive(selectedView === "projects" && projectsOverviewActive, terminalPanel)}
                onClick={parkPlaygroundThen(leaveLeftNavBuzzTermThen(onSelectProjects))}
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
              isActive={isPrimaryNavRowActive(selectedView === "agents", terminalPanel)}
              onClick={parkPlaygroundThen(leaveLeftNavBuzzTermThen(onSelectAgents))}
              tooltip="Agents"
              type="button"
            >
              <Bot className="h-4 w-4" />
              <SidebarMenuLabel>Agents</SidebarMenuLabel>
            </SidebarMenuButton>
            <SidebarMenuCountBadge
              count={counts.agents}
              preferenceEnabled={preferences.agents}
              testId="sidebar-agents-count"
            />
          </SidebarMenuItem>
          <SidebarMenuItem>
            <SidebarMenuButton
              className="data-[active=true]:font-normal"
              data-testid="open-bots-view"
              isActive={isPrimaryNavRowActive(selectedView === "bots", terminalPanel)}
              onClick={parkPlaygroundThen(leaveLeftNavBuzzTermThen(onSelectBots))}
              tooltip="Bots"
              type="button"
            >
              <BotMessageSquare className="h-4 w-4" />
              <SidebarMenuLabel>Bots</SidebarMenuLabel>
            </SidebarMenuButton>
            <SidebarMenuCountBadge
              count={counts.bots}
              preferenceEnabled={preferences.bots}
              testId="sidebar-bots-count"
            />
          </SidebarMenuItem>
          <ProtectedBestieSidebarEntry />
          <SidebarMenuItem>
            <SidebarMenuButton
              className="data-[active=true]:font-normal"
              data-testid="open-browsers-view"
              isActive={isPrimaryNavRowActive(selectedView === "browsers", terminalPanel)}
              onClick={parkPlaygroundThen(leaveLeftNavBuzzTermThen(onSelectBrowsers))}
              tooltip="Browsers"
              type="button"
            >
              <AppWindow className="h-4 w-4" />
              <SidebarMenuLabel>Browsers</SidebarMenuLabel>
            </SidebarMenuButton>
            <SidebarMenuCountBadge
              count={counts.browsers}
              legacyWhenPositive
              preferenceEnabled={preferences.browsers}
              testId="sidebar-browsers-count"
            />
          </SidebarMenuItem>
          <SidebarMenuItem>
            <SidebarMenuButton
              className="data-[active=true]:font-normal"
              data-testid="open-buzz-term-view"
              isActive={buzzTermActive}
              onClick={() => {
                // Open Term before park so pin restore sees left-nav Term
                // active and PinnedSiteSurface does not re-show on top.
                openTerminalPanel("maximized", "all");
                parkPlaygroundHost();
              }}
              tooltip="Buzz Term"
              type="button"
            >
              <SquareTerminal className="h-4 w-4" />
              <SidebarMenuLabel>Buzz Term</SidebarMenuLabel>
            </SidebarMenuButton>
          </SidebarMenuItem>
          <FeatureGate feature="workflows">
            <SidebarMenuItem>
              <SidebarMenuButton
                data-testid="open-workflows-view"
                isActive={isPrimaryNavRowActive(selectedView === "workflows", terminalPanel)}
                onClick={parkPlaygroundThen(leaveLeftNavBuzzTermThen(onSelectWorkflows))}
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
              isActive={isPrimaryNavRowActive(selectedView === "pin" && selectedPinId === pin.id, terminalPanel)}
              key={pin.id}
              onSelect={parkPlaygroundThen(leaveLeftNavBuzzTermThen(() => onSelectPinnedSite(pin.id)))}
              pin={pin}
            />
          ))}
        </SidebarMenu>
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
