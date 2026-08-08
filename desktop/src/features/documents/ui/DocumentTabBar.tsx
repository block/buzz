import { X } from "lucide-react";

import type { DocumentTab } from "@/features/documents/lib/documentTabs";
import { cn } from "@/shared/lib/cn";

export function DocumentTabBar({
  activePath,
  onActivate,
  onClose,
  tabs,
}: {
  activePath: string | null;
  onActivate: (path: string) => void;
  onClose: (path: string) => void;
  tabs: DocumentTab[];
}) {
  if (tabs.length === 0) return null;

  return (
    <div
      className="flex shrink-0 items-stretch gap-px overflow-x-auto border-b border-border/60"
      data-testid="documents-tab-bar"
    >
      {tabs.map((tab) => {
        const isActive = tab.path === activePath;
        return (
          <div
            className={cn(
              "group flex min-w-0 items-center gap-1.5 border-b-2 pl-3 pr-1.5",
              isActive
                ? "border-primary bg-background"
                : "border-transparent text-muted-foreground hover:bg-sidebar-accent/40",
            )}
            key={tab.path}
          >
            <button
              className="min-w-0 max-w-40 truncate py-2 text-left text-sm"
              data-testid={`documents-tab-${tab.name}`}
              onClick={() => onActivate(tab.path)}
              title={tab.path}
              type="button"
            >
              {tab.name}
            </button>

            {/* The dirty dot doubles as the close target's resting state, the
                way most editors do it — hovering swaps it for an ✕. */}
            <button
              aria-label={`Close ${tab.name}`}
              className="flex h-5 w-5 shrink-0 items-center justify-center rounded hover:bg-muted"
              data-testid={`documents-tab-close-${tab.name}`}
              onClick={() => onClose(tab.path)}
              type="button"
            >
              {tab.isDirty ? (
                <>
                  <span
                    aria-hidden="true"
                    className="h-1.5 w-1.5 rounded-full bg-foreground/60 group-hover:hidden"
                    data-testid={`documents-tab-dirty-${tab.name}`}
                  />
                  <X className="hidden h-3.5 w-3.5 group-hover:block" />
                </>
              ) : (
                <X className="h-3.5 w-3.5 opacity-0 group-hover:opacity-100" />
              )}
            </button>
          </div>
        );
      })}
    </div>
  );
}
