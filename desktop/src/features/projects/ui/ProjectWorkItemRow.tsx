import type * as React from "react";

import { cn } from "@/shared/lib/cn";

export function ProjectWorkItemRow({
  eventId,
  identifier,
  identifierClassName,
  identifierTitle,
  metadata,
  onOpen,
  statusIcon,
  testId,
  title,
  trailing,
}: {
  eventId: string;
  identifier: string;
  identifierClassName?: string;
  identifierTitle?: string;
  metadata?: React.ReactNode;
  onOpen?: () => void;
  statusIcon: React.ReactNode;
  testId: string;
  title: string;
  trailing?: React.ReactNode;
}) {
  return (
    <article
      className="group/work-item flex min-h-10 min-w-0 items-center gap-2 rounded-md px-2 py-2 transition-colors hover:bg-muted/30"
      data-project-event-id={eventId}
      data-testid={testId}
    >
      <span className="flex h-4 w-4 shrink-0 items-center justify-center opacity-80">
        {statusIcon}
      </span>
      {onOpen ? (
        <button
          className={cn(
            "w-[4.75rem] shrink-0 truncate rounded-sm text-left text-xs font-medium tabular-nums text-muted-foreground/70 transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
            identifierClassName,
          )}
          onClick={onOpen}
          title={identifierTitle}
          type="button"
        >
          {identifier}
        </button>
      ) : (
        <span
          className={cn(
            "w-[4.75rem] shrink-0 truncate text-xs font-medium tabular-nums text-muted-foreground/70",
            identifierClassName,
          )}
        >
          {identifier}
        </span>
      )}
      {onOpen ? (
        <button
          className="min-w-0 truncate rounded-sm text-left text-sm font-normal text-foreground transition-colors hover:text-primary focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
          onClick={onOpen}
          title={title}
          type="button"
        >
          {title}
        </button>
      ) : (
        <span className="min-w-0 truncate text-sm font-normal text-foreground">
          {title}
        </span>
      )}
      {metadata ? (
        <div className="hidden min-w-0 shrink items-center gap-1.5 overflow-hidden text-xs text-muted-foreground lg:flex">
          <span className="text-muted-foreground/45">›</span>
          {metadata}
        </div>
      ) : null}
      {trailing ? (
        <div className="ml-auto flex shrink-0 items-center gap-2 pl-2">
          {trailing}
        </div>
      ) : null}
    </article>
  );
}
