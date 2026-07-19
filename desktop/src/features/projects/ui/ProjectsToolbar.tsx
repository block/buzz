import { LayoutGrid, List } from "lucide-react";

import type {
  ProjectsFilter,
  ProjectsViewMode,
} from "@/features/projects/lib/projectsViewHelpers";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

const SELECTED_MENU_ITEM_CLASSES =
  "font-semibold text-foreground after:opacity-100 hover:text-foreground";

type ProjectsToolbarProps = {
  filter: ProjectsFilter;
  onFilterChange: (filter: ProjectsFilter) => void;
};

export function ProjectsViewModeToggle({
  viewMode,
  onViewModeChange,
}: {
  viewMode: ProjectsViewMode;
  onViewModeChange: (viewMode: ProjectsViewMode) => void;
}) {
  return (
    <fieldset className="flex items-center rounded-lg bg-muted/30 p-0.5">
      <legend className="sr-only">Project layout</legend>
      <Button
        aria-label="Grid layout"
        aria-pressed={viewMode === "grid"}
        className="h-7 w-7 px-0"
        onClick={() => onViewModeChange("grid")}
        size="xs"
        type="button"
        variant={viewMode === "grid" ? "secondary" : "ghost"}
      >
        <LayoutGrid className="h-3.5 w-3.5" />
      </Button>
      <Button
        aria-label="List layout"
        aria-pressed={viewMode === "list"}
        className="h-7 w-7 px-0"
        onClick={() => onViewModeChange("list")}
        size="xs"
        type="button"
        variant={viewMode === "list" ? "secondary" : "ghost"}
      >
        <List className="h-3.5 w-3.5" />
      </Button>
    </fieldset>
  );
}

export function ProjectsToolbar({
  filter,
  onFilterChange,
}: ProjectsToolbarProps) {
  const filterOptions: Array<{
    label: string;
    value: ProjectsFilter;
  }> = [
    { label: "Overview", value: "all" },
    { label: "Repositories", value: "repositories" },
    { label: "Pull Requests", value: "prs" },
    { label: "Issues", value: "issues" },
  ];

  return (
    <div
      className="pointer-events-auto flex h-full min-w-0 items-center"
      data-tauri-drag-region
    >
      <div className="flex h-full min-w-0 flex-1 items-center gap-0.5 overflow-hidden">
        <fieldset className="flex h-full min-w-0 flex-1 flex-nowrap items-stretch gap-1 overflow-x-auto scrollbar-none [&::-webkit-scrollbar]:hidden">
          <legend className="sr-only">Project owner filter</legend>
          {filterOptions.map((option) => (
            <Button
              aria-label={option.label}
              aria-pressed={filter === option.value}
              className={cn(
                "relative h-full shrink-0 gap-1.5 rounded-none px-2.5 text-base leading-5 tracking-tight text-muted-foreground after:absolute after:inset-x-2.5 after:bottom-0 after:h-0.5 after:bg-current after:opacity-0 after:transition-opacity after:content-[''] hover:bg-transparent hover:text-foreground hover:after:opacity-100",
                option.value === "all" && "pl-0 after:left-0",
                filter === option.value && SELECTED_MENU_ITEM_CLASSES,
              )}
              key={option.value}
              onClick={() => onFilterChange(option.value)}
              type="button"
              variant="ghost"
            >
              <span className="grid">
                <span
                  aria-hidden="true"
                  className="invisible col-start-1 row-start-1 font-semibold"
                >
                  {option.label}
                </span>
                <span className="col-start-1 row-start-1">{option.label}</span>
              </span>
            </Button>
          ))}
        </fieldset>
      </div>
    </div>
  );
}
