import { isTauri } from "@tauri-apps/api/core";
import { useNavigate } from "@tanstack/react-router";
import {
  WebRemoteWatcher,
  useWebRemoteNav,
} from "@/features/bridge/ui/WebRemoteSettings";
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
import type { ReactElement, Ref } from "react";

import type { SurfaceTabTarget } from "@/app/surfaceTabs/surfaceTabModel";

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
import { usePinnedSitesSeparateSection } from "@/features/pinned-sites/lib/separateSectionPreference";
import { TopbarSearch } from "@/features/search/ui/TopbarSearch";
import { scopedPrimaryMenuTestId } from "@/features/sidebar/lib/primaryMenu";
import { useSidebarMenuCounts } from "@/features/sidebar/lib/useSidebarMenuCounts";
import { SidebarSurfaceTabMenu } from "@/features/sidebar/ui/AddSurfaceTabMenuItem";
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

export type AppSidebarPrimaryMenuProps = {
  dragRegion?: boolean;
  headerRef?: Ref<HTMLDivElement>;
  includeProjects?: boolean;
  /** Off-screen menu stays in layout for scroll measurement, but leaves the tab order. */
  inert?: boolean;
  menuTestId?: string;
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
  testIdScope?: string;
};

function PrimaryTabRow({
  children,
  label,
  target,
}: {
  children: ReactElement;
  label: string;
  target: SurfaceTabTarget;
}) {
  return (
    <SidebarSurfaceTabMenu label={label} target={target}>
      {children}
    </SidebarSurfaceTabMenu>
  );
}

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
  dragRegion = true,
  headerRef,
  includeProjects = true,
  inert,
  menuTestId = "sidebar-primary-menu",
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
  testIdScope,
}: AppSidebarPrimaryMenuProps) {
  const { pins: allPins } = usePinnedSites();
  const web = !isTauri();
  const remoteNav = useWebRemoteNav();
  const navigate = useNavigate();
  // The Buzz term page (/buzz-term, also the instructions state) highlights Buzz term, never Agents.
  const onBuzzTermPage = web && selectedView === "buzzTerm";
  // Agents is NEVER greyed out: it always opens the Agents page (the relay-bridge list + what to enable for management).
  // Buzz term is plain greyed out while unavailable; clicking opens an instructions page.
  const terminalLocked = web && !remoteNav.terminal;
  const pins = isTauri() ? allPins : []; // web/mobile: pinned sites are desktop-only (docs/pwa-web-differences.md #1)
  const separateSection = usePinnedSitesSeparateSection();
  const { preferences, counts } = useSidebarMenuCounts();
  const terminalPanel = useTerminalPanel();
  const buzzTermActive = isLeftNavBuzzTermActive(terminalPanel);
  const rowTestId = (id: string) => scopedPrimaryMenuTestId(testIdScope, id);
  const pinRows = pins.map((pin) => (
    <PinnedSiteMenuItem
      isActive={isPrimaryNavRowActive(
        selectedView === "pin" && selectedPinId === pin.id,
        terminalPanel,
      )}
      key={pin.id}
      onSelect={parkPlaygroundThen(
        leaveLeftNavBuzzTermThen(() => onSelectPinnedSite(pin.id)),
      )}
      pin={pin}
      testId={rowTestId(`open-pinned-site-${pin.id}`)}
    />
  ));
  return (
    <>
      {web ? <WebRemoteWatcher /> : null}
      <SidebarHeader
        className="relative z-40 cursor-default select-none px-2 pb-0 pt-0"
        data-tauri-drag-region={dragRegion ? true : undefined}
        data-testid={menuTestId}
        inert={inert ? true : undefined}
        ref={headerRef}
      >
        <SidebarMenu className="sidebar-primary-menu pb-2">
          <PrimaryTabRow label="Inbox" target={{ kind: "home" }}>
            <SidebarMenuItem>
              <SidebarMenuButton
                className="data-[active=true]:font-normal"
                isActive={isPrimaryNavRowActive(
                  selectedView === "home",
                  terminalPanel,
                )}
                onClick={parkPlaygroundThen(
                  leaveLeftNavBuzzTermThen(onSelectHome),
                )}
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
                testId={rowTestId("sidebar-home-count")}
              />
            </SidebarMenuItem>
          </PrimaryTabRow>
          <FeatureGate feature="pulse">
            <PrimaryTabRow label="Pulse" target={{ kind: "pulse" }}>
              <SidebarMenuItem>
                <SidebarMenuButton
                  data-testid={rowTestId("open-pulse-view")}
                  isActive={isPrimaryNavRowActive(
                    selectedView === "pulse",
                    terminalPanel,
                  )}
                  onClick={parkPlaygroundThen(
                    leaveLeftNavBuzzTermThen(onSelectPulse),
                  )}
                  tooltip="Pulse"
                  type="button"
                >
                  <Activity className="h-4 w-4" />
                  <SidebarMenuLabel>Pulse</SidebarMenuLabel>
                </SidebarMenuButton>
              </SidebarMenuItem>
            </PrimaryTabRow>
          </FeatureGate>
          <PrimaryTabRow label="Projects" target={{ kind: "projects" }}>
            <SidebarMenuItem>
              <SidebarMenuButton
                data-testid={rowTestId("open-projects-view")}
                isActive={isPrimaryNavRowActive(
                  selectedView === "projects" && projectsOverviewActive,
                  terminalPanel,
                )}
                onClick={parkPlaygroundThen(
                  leaveLeftNavBuzzTermThen(onSelectProjects),
                )}
                tooltip="Projects"
                type="button"
              >
                <Folders className="h-4 w-4" />
                <SidebarMenuLabel>Projects</SidebarMenuLabel>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </PrimaryTabRow>
          <PrimaryTabRow label="Agents" target={{ kind: "agents" }}>
            <SidebarMenuItem>
              <SidebarMenuButton
                className="data-[active=true]:font-normal"
                data-testid={rowTestId("open-agents-view")}
                isActive={isPrimaryNavRowActive(
                  selectedView === "agents",
                  terminalPanel,
                )}
                onClick={parkPlaygroundThen(
                  leaveLeftNavBuzzTermThen(onSelectAgents),
                )}
                tooltip="Agents"
                type="button"
              >
                <Bot className="h-4 w-4" />
                <SidebarMenuLabel>Agents</SidebarMenuLabel>
              </SidebarMenuButton>
              <SidebarMenuCountBadge
                count={counts.agents}
                preferenceEnabled={preferences.agents}
                testId={rowTestId("sidebar-agents-count")}
              />
            </SidebarMenuItem>
          </PrimaryTabRow>
          <PrimaryTabRow label="Bots" target={{ kind: "bots" }}>
            <SidebarMenuItem>
              <SidebarMenuButton
                className="data-[active=true]:font-normal"
                data-testid={rowTestId("open-bots-view")}
                isActive={isPrimaryNavRowActive(
                  selectedView === "bots",
                  terminalPanel,
                )}
                onClick={parkPlaygroundThen(
                  leaveLeftNavBuzzTermThen(onSelectBots),
                )}
                tooltip="Bots"
                type="button"
              >
                <BotMessageSquare className="h-4 w-4" />
                <SidebarMenuLabel>Bots</SidebarMenuLabel>
              </SidebarMenuButton>
              <SidebarMenuCountBadge
                count={counts.bots}
                preferenceEnabled={preferences.bots}
                testId={rowTestId("sidebar-bots-count")}
              />
            </SidebarMenuItem>
          </PrimaryTabRow>
          <ProtectedBestieSidebarEntry />
          <PrimaryTabRow label="Browsers" target={{ kind: "browsers" }}>
            <SidebarMenuItem>
              <SidebarMenuButton
                className="data-[active=true]:font-normal"
                data-testid={rowTestId("open-browsers-view")}
                isActive={isPrimaryNavRowActive(
                  selectedView === "browsers",
                  terminalPanel,
                )}
                onClick={parkPlaygroundThen(
                  leaveLeftNavBuzzTermThen(onSelectBrowsers),
                )}
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
                testId={rowTestId("sidebar-browsers-count")}
              />
            </SidebarMenuItem>
          </PrimaryTabRow>
          <PrimaryTabRow
            label={web ? "Buzz term" : "Buzz Term"}
            target={{ kind: "buzz-term" }}
          >
            <SidebarMenuItem>
              <SidebarMenuButton
                className={`data-[active=true]:font-normal${terminalLocked ? " opacity-50" : ""}`}
                data-testid={rowTestId("open-buzz-term-view")}
                data-unavailable={terminalLocked ? "true" : undefined}
                isActive={buzzTermActive || onBuzzTermPage}
                onClick={() => {
                  if (web) {
                    // Always opens the Buzz term page; while unavailable it shows the instructions (row stays greyed).
                    void navigate({ to: "/buzz-term" });
                    return;
                  }
                  // Open Term before park so pin restore sees left-nav Term
                  // active and PinnedSiteSurface does not re-show on top.
                  openTerminalPanel("maximized", "all");
                  parkPlaygroundHost();
                }}
                tooltip="Buzz Term"
                type="button"
              >
                <SquareTerminal className="h-4 w-4" />
                <SidebarMenuLabel>
                  {web ? "Buzz term" : "Buzz Term"}
                </SidebarMenuLabel>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </PrimaryTabRow>
          <FeatureGate feature="workflows">
            <PrimaryTabRow label="Workflows" target={{ kind: "workflows" }}>
              <SidebarMenuItem>
                <SidebarMenuButton
                  data-testid={rowTestId("open-workflows-view")}
                  isActive={isPrimaryNavRowActive(
                    selectedView === "workflows",
                    terminalPanel,
                  )}
                  onClick={parkPlaygroundThen(
                    leaveLeftNavBuzzTermThen(onSelectWorkflows),
                  )}
                  tooltip="Workflows"
                  type="button"
                >
                  <Zap className="h-4 w-4" />
                  <SidebarMenuLabel>Workflows</SidebarMenuLabel>
                </SidebarMenuButton>
              </SidebarMenuItem>
            </PrimaryTabRow>
          </FeatureGate>
          {separateSection ? null : pinRows}
        </SidebarMenu>
        {separateSection && pins.length > 0 ? (
          <div
            className="border-t border-sidebar-border/70 pt-2 pb-2"
            data-testid={rowTestId("pinned-sites-section")}
          >
            <div className="px-2 pb-1 text-[11px] font-medium uppercase tracking-wide text-muted-foreground/70 group-data-[collapsible=icon]:hidden">
              Pinned
            </div>
            <SidebarMenu>{pinRows}</SidebarMenu>
          </div>
        ) : null}
      </SidebarHeader>
      {includeProjects ? <SidebarProjectsSection /> : null}
    </>
  );
}

function PinnedSiteMenuItem({
  isActive,
  onSelect,
  pin,
  testId,
}: {
  isActive: boolean;
  onSelect: () => void;
  pin: PinnedSite;
  testId: string;
}) {
  const Icon = getPinnedSiteIcon(pin.icon);
  return (
    <SidebarSurfaceTabMenu
      label={pin.name}
      target={{ kind: "pin", pinId: pin.id }}
    >
      <SidebarMenuItem>
        <SidebarMenuButton
          className="data-[active=true]:font-normal"
          data-testid={testId}
          isActive={isActive}
          onClick={onSelect}
          tooltip={pin.name}
          type="button"
        >
          <Icon className="h-4 w-4" />
          <SidebarMenuLabel>{pin.name}</SidebarMenuLabel>
        </SidebarMenuButton>
      </SidebarMenuItem>
    </SidebarSurfaceTabMenu>
  );
}
