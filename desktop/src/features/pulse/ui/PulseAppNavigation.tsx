import { Bot, Folders, MessageCircle, Zap } from "lucide-react";
import { cn } from "@/shared/lib/cn";
import type { PulseWorkspacePage } from "../lib/workspaceNavigation";

export type PulseApp = "messages" | PulseWorkspacePage;

const apps = {
  messages: { label: "Messages", icon: MessageCircle },
  projects: { label: "Projects", icon: Folders },
  agents: { label: "Agents", icon: Bot },
  workflows: { label: "Workflows", icon: Zap },
} as const;

/** Window-level app switching, separate from each app's own navigation. */
export function PulseAppNavigation({
  active,
  onSelect,
}: {
  active: PulseApp;
  onSelect: (app: PulseApp) => void;
}) {
  return (
    <nav
      aria-label="Apps"
      data-testid="pulse-app-navigation"
      className="w-[64px] shrink-0 space-y-1 overflow-y-auto px-2 py-2 lg:w-[180px]"
    >
      {(Object.keys(apps) as PulseApp[]).map((app) => {
        const Icon = apps[app].icon;
        return (
          <button
            key={app}
            type="button"
            aria-label={apps[app].label}
            aria-current={active === app ? "page" : undefined}
            onClick={() => onSelect(app)}
            className={cn(
              "flex w-full items-center justify-center gap-3 rounded-xl px-3 py-3 text-left text-sm hover:bg-muted/35 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring lg:justify-start",
              active === app
                ? "font-semibold text-foreground"
                : "text-muted-foreground",
            )}
          >
            <Icon aria-hidden="true" className="h-4 w-4 shrink-0" />
            <span className="hidden lg:inline">{apps[app].label}</span>
          </button>
        );
      })}
    </nav>
  );
}
