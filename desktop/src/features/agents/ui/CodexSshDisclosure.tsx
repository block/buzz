import { ChevronDown } from "lucide-react";
import type * as React from "react";

export function CodexSshDisclosure({
  children,
  connected,
  expanded,
  onExpandedChange,
}: {
  children: React.ReactNode;
  connected: boolean;
  expanded: boolean;
  onExpandedChange: (expanded: boolean) => void;
}) {
  return (
    <section className="rounded-md border border-border/60">
      <button
        aria-expanded={expanded}
        className="flex w-full items-center gap-3 px-3 py-2.5 text-left hover:bg-muted/40"
        data-testid="codex-ssh-disclosure"
        onClick={() => onExpandedChange(!expanded)}
        type="button"
      >
        <span className="min-w-0 flex-1">
          <span className="block text-sm font-medium">
            Remote Codex computer (SSH)
          </span>
          <span className="block text-xs text-muted-foreground">
            {connected ? "Remote runtime connected" : "Optional"}
          </span>
        </span>
        <ChevronDown
          aria-hidden="true"
          className={`h-4 w-4 shrink-0 text-muted-foreground transition-transform ${expanded ? "rotate-180" : ""}`}
        />
      </button>
      {expanded ? (
        <div
          className="space-y-3 border-t border-border/60 p-3"
          data-testid="codex-ssh-content"
        >
          {children}
        </div>
      ) : null}
    </section>
  );
}
