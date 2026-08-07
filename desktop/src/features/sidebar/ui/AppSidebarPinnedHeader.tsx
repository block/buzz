import { Activity, Bot, FolderGit2, Inbox, Zap } from "lucide-react";
import { useTranslation } from "react-i18next";

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

type AppSidebarPinnedHeaderProps = {
  channelLabels: Record<string, string>;
  currentPubkey?: string;
  onBrowseChannels?: () => void;
  onCreateAgent: () => void;
  onCreateChannel: () => void;
  onOpenDm: (input: { pubkeys: string[] }) => Promise<void>;
  onOpenSearchResult: (hit: SearchHit) => void;
  onSelectChannel: (channelId: string) => void;
  searchChannels: Channel[];
  searchFocusRequest: number;
  suggestionChannels: Channel[];
};

type AppSidebarPrimaryMenuProps = {
  homeBadgeCount: number;
  onSelectAgents: () => void;
  onSelectHome: () => void;
  onSelectProjects: () => void;
  onSelectPulse: () => void;
  onSelectWorkflows: () => void;
  selectedView: SidebarSelectedView;
};

export function AppSidebarPinnedHeader({
  channelLabels,
  currentPubkey,
  onBrowseChannels,
  onCreateAgent,
  onCreateChannel,
  onOpenDm,
  onOpenSearchResult,
  onSelectChannel,
  searchChannels,
  searchFocusRequest,
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
        currentPubkey={currentPubkey}
        focusRequest={searchFocusRequest}
        onOpenChannel={onSelectChannel}
        onOpenResult={onOpenSearchResult}
        onOpenUser={(user) => onOpenDm({ pubkeys: [user.pubkey] })}
        onBrowseChannels={onBrowseChannels}
        onCreateAgent={onCreateAgent}
        onCreateChannel={onCreateChannel}
        suggestionChannels={suggestionChannels}
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
  const { t } = useTranslation();
  return (
    <SidebarHeader
      className="relative z-40 cursor-default select-none px-2 pb-0 pt-0"
      data-tauri-drag-region
      data-testid="sidebar-primary-menu"
    >
      <SidebarMenu className="pb-2">
        <SidebarMenuItem>
          <SidebarMenuButton
            className="data-[active=true]:font-normal"
            isActive={selectedView === "home"}
            onClick={onSelectHome}
            tooltip={t("sidebar.primary.inbox")}
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
              {t("sidebar.primary.inbox")}
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
              tooltip={t("sidebar.primary.pulse")}
              type="button"
            >
              <Activity className="h-4 w-4" />
              <SidebarMenuLabel>{t("sidebar.primary.pulse")}</SidebarMenuLabel>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </FeatureGate>
        <FeatureGate feature="projects">
          <SidebarMenuItem>
            <SidebarMenuButton
              data-testid="open-projects-view"
              isActive={selectedView === "projects"}
              onClick={onSelectProjects}
              tooltip={t("sidebar.primary.projects")}
              type="button"
            >
              <FolderGit2 className="h-4 w-4" />
              <SidebarMenuLabel>
                {t("sidebar.primary.projects")}
              </SidebarMenuLabel>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </FeatureGate>
        <SidebarMenuItem>
          <SidebarMenuButton
            className="data-[active=true]:font-normal"
            data-testid="open-agents-view"
            isActive={selectedView === "agents"}
            onClick={onSelectAgents}
            tooltip={t("sidebar.primary.agents")}
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
              {t("sidebar.primary.agents")}
            </SidebarMenuLabel>
          </SidebarMenuButton>
        </SidebarMenuItem>
        <FeatureGate feature="workflows">
          <SidebarMenuItem>
            <SidebarMenuButton
              data-testid="open-workflows-view"
              isActive={selectedView === "workflows"}
              onClick={onSelectWorkflows}
              tooltip={t("sidebar.primary.workflows")}
              type="button"
            >
              <Zap className="h-4 w-4" />
              <SidebarMenuLabel>
                {t("sidebar.primary.workflows")}
              </SidebarMenuLabel>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </FeatureGate>
      </SidebarMenu>
    </SidebarHeader>
  );
}
