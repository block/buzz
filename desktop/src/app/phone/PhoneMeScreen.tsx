// Phone "Me" tab: user card plus entry points for pages without their own tab. Settings opens the existing (web
// read-only for desktop-owned items) Settings screen.
import { Blocks, Bot, BotMessageSquare, ChevronRight, FolderKanban, Globe, Settings, Workflow } from "lucide-react";
import type * as React from "react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";

type Row = { id: string; label: string; Icon: React.ComponentType<{ className?: string }>; onSelect: () => void };

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
  const rows: Row[] = [
    { id: "settings", label: "Settings", Icon: Settings, onSelect: onSettings },
    { id: "agents", label: "Agents", Icon: BotMessageSquare, onSelect: onAgents },
    { id: "projects", label: "Projects", Icon: FolderKanban, onSelect: onProjects },
    { id: "bots", label: "Bots", Icon: Bot, onSelect: onBots },
    { id: "browsers", label: "Browsers", Icon: Globe, onSelect: onBrowsers },
    { id: "workflows", label: "Workflows", Icon: Workflow, onSelect: onWorkflows },
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
        {rows.map(({ id, label, Icon, onSelect }) => (
          <li key={id}>
            <button
              className="flex w-full items-center gap-3 px-4 py-3.5 text-left text-[15px] active:bg-accent"
              data-testid={`phone-me-${id}`}
              onClick={onSelect}
              type="button"
            >
              <Icon className="h-5 w-5 text-muted-foreground" />
              <span className="flex-1">{label}</span>
              <ChevronRight className="h-4 w-4 text-muted-foreground" />
            </button>
          </li>
        ))}
      </ul>
      <p className="flex items-center gap-1.5 px-5 pt-4 text-xs text-muted-foreground">
        <Blocks className="h-3.5 w-3.5" /> Desktop-owned settings are read-only on the web.
      </p>
    </div>
  );
}
