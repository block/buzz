import type * as React from "react";

import type { ProjectSelectionItem } from "@/features/projects/lib/projectSelection";
import { useProjectSelection } from "@/features/projects/lib/useProjectSelection";
import { cn } from "@/shared/lib/cn";
import { ProjectEntitySelectControl } from "./ProjectEntityListRow";

export function ProjectWorkItemRow({
  eventId,
  identifier,
  identifierClassName,
  identifierTitle,
  metadata,
  onOpen,
  selection,
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
  selection?: {
    item: ProjectSelectionItem;
    rangeItems: ProjectSelectionItem[];
  };
  statusIcon: React.ReactNode;
  testId: string;
  title: string;
  trailing?: React.ReactNode;
}) {
  const projectSelection = useProjectSelection();
  const selected = Boolean(
    selection && projectSelection?.isSelected(selection.item.id),
  );
  const showSelectControl = Boolean(selection && projectSelection && selected);

  return (
    <article
      className={cn(
        "group/work-item mx-2 flex min-h-10 min-w-0 items-center gap-3 rounded-md px-2 py-2 transition-colors hover:bg-muted/30",
        selected && "bg-muted/40",
      )}
      data-project-event-id={eventId}
      data-selected={selected ? "true" : undefined}
      data-testid={testId}
    >
      <span
        className="relative flex h-4 w-4 shrink-0 items-center justify-center"
        data-testid="project-work-item-leading-icon"
      >
        <span
          className={cn(
            "flex items-center justify-center opacity-80 transition-opacity",
            selection &&
              "group-hover/work-item:opacity-0 group-focus-within/work-item:opacity-0",
            showSelectControl && "opacity-0",
          )}
          data-testid="project-work-item-status-icon"
        >
          {statusIcon}
        </span>
        {selection && projectSelection ? (
          <span
            className={cn(
              "absolute inset-0 flex items-center justify-center opacity-0 transition-opacity group-hover/work-item:opacity-100 group-focus-within/work-item:opacity-100",
              showSelectControl && "opacity-100",
            )}
          >
            <ProjectEntitySelectControl
              checked={selected}
              label={`Select ${selection.item.title}`}
              onToggle={({ shiftKey }) =>
                projectSelection.toggle(selection.item, {
                  rangeItems: selection.rangeItems,
                  shiftKey,
                })
              }
            />
          </span>
        ) : null}
      </span>
      {onOpen ? (
        <button
          className="min-w-0 flex-1 truncate rounded-sm text-left text-sm font-normal text-foreground transition-colors hover:text-primary focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
          data-projects-text-priority="primary"
          onClick={onOpen}
          title={title}
          type="button"
        >
          {title}
        </button>
      ) : (
        <span
          className="min-w-0 flex-1 truncate text-sm font-normal text-foreground"
          data-projects-text-priority="primary"
        >
          {title}
        </span>
      )}
      {onOpen ? (
        <button
          className={cn(
            "w-[4.75rem] shrink-0 truncate rounded-sm text-left text-xs font-medium tabular-nums text-muted-foreground/55 transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
            identifierClassName,
          )}
          data-projects-text-priority="secondary"
          data-testid="project-work-item-identifier"
          onClick={onOpen}
          title={identifierTitle}
          type="button"
        >
          {identifier}
        </button>
      ) : (
        <span
          className={cn(
            "w-[4.75rem] shrink-0 truncate text-xs font-medium tabular-nums text-muted-foreground/55",
            identifierClassName,
          )}
          data-projects-text-priority="secondary"
          data-testid="project-work-item-identifier"
        >
          {identifier}
        </span>
      )}
      {metadata ? (
        <div
          className="hidden min-w-0 shrink items-center gap-1.5 overflow-hidden text-xs text-muted-foreground/60 lg:flex"
          data-projects-text-priority="secondary"
        >
          <span className="text-muted-foreground/45">›</span>
          {metadata}
        </div>
      ) : null}
      {trailing ? (
        <div
          className="ml-auto flex shrink-0 items-center gap-2 pl-2 text-muted-foreground/60"
          data-projects-text-priority="secondary"
        >
          {trailing}
        </div>
      ) : null}
    </article>
  );
}
