import * as React from "react";

import type { ProjectSelectionItem } from "@/features/projects/lib/projectSelection";
import { useProjectSelection } from "@/features/projects/lib/useProjectSelection";
import { cn } from "@/shared/lib/cn";
import { ProjectEntitySelectControl } from "./ProjectEntityListRow";

export function ProjectSelectableGroup({
  children,
  contentClassName,
  count,
  groupKey,
  headerTestId,
  icon,
  items,
  label,
  labelTestId,
  testId,
}: {
  children: React.ReactNode;
  contentClassName?: string;
  count: number;
  groupKey: string;
  headerTestId: string;
  icon: React.ReactNode;
  items: ProjectSelectionItem[];
  label: string;
  labelTestId?: string;
  testId: string;
}) {
  const [expanded, setExpanded] = React.useState(true);
  const selection = useProjectSelection();
  const selectedCount =
    selection?.items.filter((selected) =>
      items.some((item) => item.id === selected.id),
    ).length ?? 0;
  const allSelected = items.length > 0 && selectedCount === items.length;
  const partiallySelected = selectedCount > 0 && !allSelected;

  return (
    <section
      className="pt-2 first:pt-0"
      data-project-group={groupKey}
      data-testid={testId}
    >
      <div
        className="group/header mx-2 flex h-9 items-center gap-1.5 rounded-md bg-muted/40 px-2 text-xs text-muted-foreground"
        data-testid={headerTestId}
      >
        {selection && items.length > 0 ? (
          <span
            className={cn(
              "flex shrink-0 opacity-0 transition-opacity group-focus-within/header:opacity-100 group-hover/header:opacity-100",
              (allSelected || partiallySelected) && "opacity-100",
            )}
          >
            <ProjectEntitySelectControl
              checked={allSelected}
              indeterminate={partiallySelected}
              label={`${allSelected ? "Clear" : "Select"} all ${label}`}
              onToggle={() => selection.toggleGroup(items)}
              testId="projects-group-select"
            />
          </span>
        ) : null}
        <button
          aria-expanded={expanded}
          className="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5 text-left"
          onClick={() => setExpanded((current) => !current)}
          type="button"
        >
          <span className="flex h-4 w-4 shrink-0 items-center justify-center opacity-80">
            {icon}
          </span>
          <span
            className="min-w-0 truncate font-medium text-foreground/80"
            data-testid={labelTestId}
          >
            {label}
          </span>
          {!expanded ? (
            <span className="shrink-0 tabular-nums text-muted-foreground/65">
              {count}
            </span>
          ) : null}
        </button>
      </div>
      {expanded ? (
        <div className={cn("mt-1 space-y-0.5", contentClassName)}>
          {children}
        </div>
      ) : null}
    </section>
  );
}
