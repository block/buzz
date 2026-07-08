import type * as React from "react";

/**
 * Shared list-row scaffold for the project detail feeds (commits, pull
 * requests, and issues) so every entry has the same structure: a clickable
 * title line with an optional status icon, a meta line underneath, and a
 * trailing cluster (hash, id, comment count) on the right.
 */
export function ProjectFeedRow({
  meta,
  onOpen,
  statusIcon,
  testId,
  title,
  trailing,
}: {
  meta: React.ReactNode;
  onOpen?: () => void;
  statusIcon?: React.ReactNode;
  testId?: string;
  title: string;
  trailing?: React.ReactNode;
}) {
  return (
    <article
      className="group/feed-item flex min-w-0 items-center justify-between gap-3 p-3 transition-colors hover:bg-muted/35"
      data-testid={testId}
    >
      <div className="min-w-0 flex-1 space-y-1">
        <div className="flex min-w-0 items-center gap-1.5">
          {onOpen ? (
            <button
              className="block min-w-0 truncate rounded-sm text-left text-sm font-semibold leading-5 text-foreground transition-colors hover:text-primary hover:underline focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
              onClick={onOpen}
              title={title}
              type="button"
            >
              {title}
            </button>
          ) : (
            <p className="truncate text-sm font-semibold leading-5 text-foreground">
              {title}
            </p>
          )}
          {statusIcon}
        </div>
        <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs leading-4 text-muted-foreground">
          {meta}
        </div>
      </div>
      {trailing ? (
        <div className="flex shrink-0 items-center gap-2">{trailing}</div>
      ) : null}
    </article>
  );
}

/** Bordered trailing cluster shared by feed rows (hash + copy, #id, …). */
export function ProjectFeedRowCluster({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div className="flex shrink-0 items-center divide-x divide-border/60 overflow-hidden border border-border/60 bg-background/45">
      {children}
    </div>
  );
}

/** Monospace cell inside a feed row cluster; a button when onClick is set. */
export function ProjectFeedRowMonoCell({
  label,
  onClick,
  title,
}: {
  label: string;
  onClick?: () => void;
  title?: string;
}) {
  if (onClick) {
    return (
      <button
        className="h-7 px-2 font-mono text-xs text-muted-foreground transition-colors hover:bg-muted/40 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        onClick={onClick}
        title={title}
        type="button"
      >
        {label}
      </button>
    );
  }
  return (
    <span className="flex h-7 items-center px-2 font-mono text-xs text-muted-foreground">
      {label}
    </span>
  );
}
