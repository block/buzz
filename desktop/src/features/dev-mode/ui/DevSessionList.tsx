import * as React from "react";

import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

function formatRelativeTime(iso: string | null) {
  if (!iso) return "";
  const deltaSeconds = Math.max(
    0,
    Math.floor((Date.now() - new Date(iso).getTime()) / 1_000),
  );
  if (deltaSeconds < 60) return "now";
  if (deltaSeconds < 3_600) return `${Math.floor(deltaSeconds / 60)}m`;
  if (deltaSeconds < 86_400) return `${Math.floor(deltaSeconds / 3_600)}h`;
  return `${Math.floor(deltaSeconds / 86_400)}d`;
}

export function DevSessionList({
  sessions,
  activeSessionId,
  onSelect,
}: {
  /** Ascending by recency — the newest session renders last, nearest the composer. */
  sessions: Channel[];
  activeSessionId: string | null;
  onSelect: (channelId: string) => void;
}) {
  // Callback ref mounts only on the active row, so selection changes scroll
  // the newly active row into view without an effect.
  const scrollActiveIntoView = React.useCallback(
    (node: HTMLButtonElement | null) => {
      node?.scrollIntoView({ block: "nearest" });
    },
    [],
  );

  if (sessions.length === 0) {
    return null;
  }

  return (
    <div
      className="max-h-56 shrink-0 overflow-y-auto border-b border-border/60 px-2 py-2 font-mono"
      data-testid="dev-mode-sessions"
    >
      {sessions.map((session) => {
        const isActive = session.id === activeSessionId;
        return (
          <button
            key={session.id}
            ref={isActive ? scrollActiveIntoView : undefined}
            className={cn(
              "flex w-full cursor-pointer items-baseline gap-2 rounded-sm px-2 py-0.5 text-left text-sm",
              isActive
                ? "bg-primary/15 text-foreground"
                : "text-muted-foreground hover:bg-muted/40 hover:text-foreground",
            )}
            onClick={() => onSelect(session.id)}
            type="button"
          >
            <span aria-hidden className="w-3 shrink-0 select-none">
              {isActive ? "▸" : ""}
            </span>
            <span className="shrink-0 font-medium"># {session.name}</span>
            <span className="min-w-0 flex-1 truncate text-muted-foreground/60">
              {session.description}
            </span>
            <span className="shrink-0 text-xs text-muted-foreground/60">
              {formatRelativeTime(session.lastMessageAt)}
            </span>
          </button>
        );
      })}
    </div>
  );
}
