// Phone "Me" tab: user card plus entry points for pages without their own tab. Settings opens the existing (web
// read-only for desktop-owned items) Settings screen.
import { Blocks, Bot, BotMessageSquare, ChevronRight, FolderKanban, Globe, Settings, Workflow } from "lucide-react";
import type * as React from "react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import {
  resolveSidebarMenuCount,
  shouldShowSidebarMenuCount,
} from "@/features/sidebar/lib/sidebarMenuCounts";
import { useSidebarMenuCounts } from "@/features/sidebar/lib/useSidebarMenuCounts";
import { FeatureGate } from "@/shared/features";

type Row = {
  id: string;
  label: string;
  Icon: React.ComponentType<{ className?: string }>;
  onSelect: () => void;
  /** Sidebar count (Appearance "Show counts on menu items"), when this row has one. */
  count?: { value: number | string | undefined; enabled: boolean };
  feature?: string;
};

export function PhoneMeScreen({
  displayName,
  avatarUrl,
  pubkey,
  onSettings,
  onAgents,
  onBots,
  onBrowsers,
  onProjects,
  onWorkflows,
}: {
  displayName: string;
  avatarUrl?: string | null;
  pubkey?: string;
  onSettings: () => void;
  onAgents: () => void;
  onBots: () => void;
  onBrowsers: () => void;
  onProjects: () => void;
  onWorkflows: () => void;
}) {
  const { preferences, counts } = useSidebarMenuCounts();
  // Same order as the desktop left-nav primary menu (Projects, Agents, Bots, Browsers, Workflows); Settings last,
  // where the desktop sidebar keeps it (footer / profile menu).
  const rows: Row[] = [
    { id: "projects", label: "Projects", Icon: FolderKanban, onSelect: onProjects },
    {
      id: "agents",
      label: "Agents",
      Icon: BotMessageSquare,
      onSelect: onAgents,
      count: { value: counts.agents, enabled: preferences.agents },
    },
    { id: "bots", label: "Bots", Icon: Bot, onSelect: onBots, count: { value: counts.bots, enabled: preferences.bots } },
    {
      id: "browsers",
      label: "Browsers",
      Icon: Globe,
      onSelect: onBrowsers,
      count: { value: counts.browsers, enabled: preferences.browsers },
    },
    { id: "workflows", label: "Workflows", Icon: Workflow, onSelect: onWorkflows, feature: "workflows" },
    { id: "settings", label: "Settings", Icon: Settings, onSelect: onSettings },
  ];
  const npubShort = pubkey ? `${pubkey.slice(0, 8)}…${pubkey.slice(-6)}` : "";
  return (
    <div className="phone-screen flex h-full flex-col overflow-y-auto bg-background" data-testid="phone-me-screen">
      <div className="flex items-center gap-3 px-4 pb-4 pt-6">
        <ProfileAvatar
          avatarUrl={avatarUrl ?? null}
          className="h-14 w-14 text-lg"
          label={displayName}
          pubkey={pubkey}
        />
        <div className="min-w-0">
          <div className="truncate text-lg font-semibold">{displayName}</div>
          {npubShort ? <div className="truncate font-mono text-xs text-muted-foreground">{npubShort}</div> : null}
        </div>
      </div>
      <ul className="mx-3 divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
        {rows.map(({ id, label, Icon, onSelect, count, feature }) => {
          const showCount =
            count != null &&
            shouldShowSidebarMenuCount({ preferenceEnabled: count.enabled, count: count.value });
          const countLabel =
            count == null ? undefined : typeof count.value === "string" ? count.value : resolveSidebarMenuCount(count.value);
          const row = (
            <li key={id}>
              <button
                className="flex w-full items-center gap-3 px-4 py-3.5 text-left text-[15px] active:bg-accent"
                data-testid={`phone-me-${id}`}
                onClick={onSelect}
                type="button"
              >
                <Icon className="h-5 w-5 text-muted-foreground" />
                <span className="flex-1">{label}</span>
                {showCount && countLabel !== undefined ? (
                  <span
                    className="rounded-full bg-primary/15 px-2 py-0.5 text-xs font-medium text-primary"
                    data-testid={`phone-me-${id}-count`}
                  >
                    {countLabel}
                  </span>
                ) : null}
                <ChevronRight className="h-4 w-4 text-muted-foreground" />
              </button>
            </li>
          );
          return feature ? (
            <FeatureGate feature={feature} key={id}>
              {row}
            </FeatureGate>
          ) : (
            row
          );
        })}
      </ul>
      <p className="flex items-center gap-1.5 px-5 pt-4 text-xs text-muted-foreground">
        <Blocks className="h-3.5 w-3.5" /> Desktop-owned settings are read-only on the web.
      </p>
    </div>
  );
}
