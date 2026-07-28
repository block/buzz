import { AppWindow, LayoutGrid, MessageSquare } from "lucide-react";
import type { ReactNode } from "react";

import type { ChannelMcpAppInstallation } from "@/features/mcp-apps/lib/channelMcpAppStorage";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

type ChannelMcpAppTabsProps = {
  activeAppId: string | null;
  apps: ChannelMcpAppInstallation[];
  onActivateApp: (appId: string) => void;
  onOpenApps: () => void;
  onShowChat: () => void;
};

function TabButton({
  active,
  children,
  onClick,
}: {
  active: boolean;
  children: ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      aria-pressed={active}
      className={cn(
        "flex h-8 shrink-0 items-center gap-1.5 rounded-lg px-2.5 text-xs font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
        active && "bg-accent text-foreground",
      )}
      onClick={onClick}
      type="button"
    >
      {children}
    </button>
  );
}

export function ChannelMcpAppTabs({
  activeAppId,
  apps,
  onActivateApp,
  onOpenApps,
  onShowChat,
}: ChannelMcpAppTabsProps) {
  return (
    <nav
      aria-label="Channel views"
      className="flex min-w-0 items-center gap-1 overflow-x-auto"
    >
      <TabButton active={activeAppId === null} onClick={onShowChat}>
        <MessageSquare className="h-3.5 w-3.5" />
        Chat
      </TabButton>
      {apps.map((app) => (
        <TabButton
          active={activeAppId === app.id}
          key={app.id}
          onClick={() => onActivateApp(app.id)}
        >
          <AppWindow className="h-3.5 w-3.5" />
          <span className="max-w-28 truncate">{app.title}</span>
        </TabButton>
      ))}
      <Button
        aria-label="Add a channel app"
        className="h-8 shrink-0 gap-1.5 px-2.5"
        onClick={onOpenApps}
        size="sm"
        type="button"
        variant="outline"
      >
        <LayoutGrid className="h-3.5 w-3.5" />
        Apps
      </Button>
    </nav>
  );
}
