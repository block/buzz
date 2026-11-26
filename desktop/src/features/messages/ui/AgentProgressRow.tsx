import * as React from "react";
import { ChevronDown, Loader2 } from "lucide-react";

import {
  agentProgressIsActive,
  agentProgressLatestLabel,
} from "@/features/messages/lib/agentProgressMessages";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { useNow } from "@/shared/lib/useNow";
import { cn } from "@/shared/lib/cn";

type AgentProgressRowProps = {
  entries: MainTimelineEntry[];
  profiles?: UserProfileLookup;
};

export function AgentProgressRow({ entries, profiles }: AgentProgressRowProps) {
  const [open, setOpen] = React.useState(false);
  const nowSeconds = Math.floor(useNow(15_000) / 1000);
  const newest = entries[entries.length - 1];
  if (!newest) {
    return null;
  }

  const bodies = entries.map((entry) => entry.message.body);
  const latest = agentProgressLatestLabel(bodies);
  const active = agentProgressIsActive(
    newest.message.body,
    newest.message.createdAt,
    nowSeconds,
  );
  const author = resolveUserLabel({
    pubkey: newest.message.pubkey ?? "",
    fallbackName: newest.message.author,
    profiles,
  });

  return (
    <div className="px-4 py-1.5" data-testid="agent-progress-row">
      <button
        className="flex w-full items-center gap-2 rounded-lg border border-border/50 bg-muted/40 px-3 py-2 text-left text-sm text-muted-foreground"
        data-testid="agent-progress-toggle"
        onClick={() => setOpen((value) => !value)}
        type="button"
      >
        <Loader2
          className={cn(
            "size-3.5 shrink-0",
            active && "animate-spin text-foreground",
          )}
        />
        <span className="min-w-0 flex-1 truncate text-foreground">
          {active
            ? `${author} is working`
            : `${author} · ${entries.length} updates`}
          <span className="text-muted-foreground">
            {" · "}
            {latest}
          </span>
        </span>
        <ChevronDown
          className={cn(
            "size-3.5 shrink-0 transition-transform",
            open && "rotate-180",
          )}
        />
      </button>
      {open ? (
        <ol className="mt-1.5 space-y-0.5 px-3 text-2xs text-muted-foreground">
          {entries.map((entry) => (
            <li className="truncate" key={entry.message.id}>
              {entry.message.body.trim()}
            </li>
          ))}
        </ol>
      ) : null}
    </div>
  );
}
