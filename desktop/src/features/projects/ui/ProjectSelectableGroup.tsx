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
  headerClassName,
  headerTestId,
  icon,
  items,
  label,
  labelClassName,
  labelTestId,
  testId,
}: {
  children: React.ReactNode;
  contentClassName?: string;
  count: number;
  groupKey: string;
  headerClassName?: string;
  headerTestId: string;
  icon: React.ReactNode;
  items: ProjectSelectionItem[];
  label: string;
  labelClassName?: string;
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
  const showSelectControl = allSelected || partiallySelected;

  return (
    <section
      className="pt-2 first:pt-0"
      data-project-group={groupKey}
      data-project-group-size={items.length}
      data-testid={testId}
    >
      <div
        className={cn(
          "group/header mx-2 flex h-9 items-center gap-1.5 rounded-md bg-muted/40 px-2 text-xs text-muted-foreground",
          headerClassName,
        )}
        data-testid={headerTestId}
      >
        <span
          className="relative flex h-4 w-4 shrink-0 items-center justify-center"
          data-testid="project-group-leading-icon"
        >
          <span
            className={cn(
              "flex items-center justify-center transition-opacity",
              selection &&
                "group-focus-within/header:opacity-0 group-hover/header:opacity-0",
              showSelectControl && "opacity-0",
            )}
            data-testid="project-group-icon"
          >
            {icon}
          </span>
          {selection && items.length > 0 ? (
            <span
              className={cn(
                "absolute inset-0 flex items-center justify-center opacity-0 transition-opacity group-focus-within/header:opacity-100 group-hover/header:opacity-100",
                showSelectControl && "opacity-100",
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
        </span>
        <button
          aria-expanded={expanded}
          className="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5 text-left"
          onClick={() => setExpanded((current) => !current)}
          type="button"
        >
          <span
            className={cn(
              "min-w-0 truncate font-medium text-muted-foreground",
              labelClassName,
            )}
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
