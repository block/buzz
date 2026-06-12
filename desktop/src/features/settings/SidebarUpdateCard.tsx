import { CircleArrowUp, X } from "lucide-react";

import { Button } from "@/shared/ui/button";

import { useUpdaterContext } from "./hooks/UpdaterProvider";
import { shouldShowSidebarUpdateCard } from "./sidebarUpdateCardVisibility";

type SidebarUpdateCardProps = {
  onDismiss: () => void;
};

export function SidebarUpdateCard({ onDismiss }: SidebarUpdateCardProps) {
  const { status, relaunch } = useUpdaterContext();

  if (!shouldShowSidebarUpdateCard(status)) {
    return null;
  }

  return (
    <div
      className="group/update-card relative w-full overflow-hidden rounded-xl bg-secondary/85 text-secondary-foreground shadow-[1px_0_0_hsl(var(--border)/0.55),-1px_0_0_hsl(var(--border)/0.55),0_1px_0_hsl(var(--border)/0.55)] dark:bg-secondary/70"
      data-testid="sidebar-update-card"
    >
      <div className="flex h-24 items-center justify-center bg-background/55 text-muted-foreground dark:bg-background/50">
        <CircleArrowUp aria-hidden="true" className="h-12 w-12" />
      </div>
      <button
        aria-label="Dismiss update notification"
        className="pointer-events-none absolute right-2 top-2 flex h-6 w-6 items-center justify-center rounded-full bg-muted-foreground/10 text-muted-foreground/70 opacity-0 shadow-xs transition-colors transition-opacity hover:bg-muted-foreground/15 hover:text-muted-foreground focus-visible:pointer-events-auto focus-visible:opacity-100 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-muted-foreground/40 group-hover/update-card:pointer-events-auto group-hover/update-card:opacity-100"
        data-testid="sidebar-update-dismiss"
        onClick={onDismiss}
        type="button"
      >
        <X aria-hidden="true" className="h-4 w-4" />
      </button>

      <div className="min-w-0 bg-sidebar-border/15 px-3 pb-3 pt-2.5 dark:bg-background/30">
        <p className="text-sm font-semibold leading-tight">Update ready</p>
        <p className="mt-1 text-xs leading-snug text-secondary-foreground/70">
          Restart to apply the update.
        </p>
        <Button
          aria-label="Restart now to apply update"
          className="mt-3 h-7 rounded-md px-2 text-xs"
          data-testid="sidebar-update-restart"
          onClick={() => {
            void relaunch();
          }}
          size="sm"
          type="button"
        >
          Restart now
        </Button>
      </div>
    </div>
  );
}
