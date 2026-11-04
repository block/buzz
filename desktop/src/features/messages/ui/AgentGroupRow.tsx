import * as React from "react";
import { ChevronRight } from "lucide-react";

import { formatTime } from "@/features/messages/lib/dateFormatters";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import { cn } from "@/shared/lib/cn";
import { UserAvatar } from "@/shared/ui/UserAvatar";

export function AgentGroupRow({
  entries,
  renderEntry,
}: {
  entries: MainTimelineEntry[];
  renderEntry: (entry: MainTimelineEntry) => React.ReactNode;
}) {
  const [expanded, setExpanded] = React.useState(false);
  const authors = [
    ...new Map(
      entries.map(({ message }) => [message.author, message]),
    ).values(),
  ];
  const first = entries[0].message;
  const last = entries[entries.length - 1].message;
  const names =
    authors.length > 2
      ? `${authors[0].author}, ${authors[1].author} +${authors.length - 2}`
      : authors.map((author) => author.author).join(", ");
  const label = `${names} · ${entries.length} agent updates · ${formatTime(first.createdAt)}–${formatTime(last.createdAt)}`;

  return (
    <div>
      <button
        aria-expanded={expanded}
        className="mx-1 flex h-7 w-[calc(100%-0.5rem)] items-center gap-2.5 rounded-lg px-2 text-left text-xs text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        data-testid="agent-group-row"
        onClick={() => setExpanded((value) => !value)}
        type="button"
      >
        <span className="flex min-w-9 shrink-0 -space-x-1">
          {authors.slice(0, 3).map(({ author, avatarUrl }) => (
            <UserAvatar
              accent
              avatarUrl={avatarUrl ?? null}
              className="ring-2 ring-background"
              displayName={author}
              key={author}
              shape="squircle"
              size="xs"
            />
          ))}
        </span>
        <span className="min-w-0 truncate">{label}</span>
        <ChevronRight
          aria-hidden="true"
          className={cn(
            "h-3.5 w-3.5 shrink-0 transition-transform",
            expanded && "rotate-90",
          )}
        />
      </button>
      {expanded
        ? entries.map((entry) => (
            <React.Fragment key={entry.message.renderKey ?? entry.message.id}>
              {renderEntry(entry)}
            </React.Fragment>
          ))
        : null}
    </div>
  );
}
