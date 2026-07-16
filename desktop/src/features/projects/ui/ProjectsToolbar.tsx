import { LayoutGrid, List } from "lucide-react";

import type {
  ProjectsFilter,
  ProjectsViewMode,
} from "@/features/projects/lib/projectsViewHelpers";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

const SELECTED_MENU_ITEM_CLASSES =
  "bg-sidebar-active text-sidebar-active-foreground shadow-xs hover:bg-sidebar-active hover:text-sidebar-active-foreground";

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
    compactLabel?: string;
    label: string;
    value: ProjectsFilter;
  }> = [
    { label: "Overview", value: "all" },
    {
      compactLabel: "Repos",
      label: "Repositories",
      value: "repositories",
    },
    { compactLabel: "PRs", label: "Pull Requests", value: "prs" },
    { label: "Issues", value: "issues" },
    { compactLabel: "Mine", label: "My Repositories", value: "mine" },
    { label: "Local", value: "local" },
  ];

  return (
    <div
      className="pointer-events-auto flex min-h-[3.25rem] min-w-0 items-center px-4 py-2"
      data-tauri-drag-region
    >
      <div className="flex min-w-0 flex-1 items-center gap-0.5 overflow-hidden">
        <fieldset className="flex min-w-0 flex-1 flex-nowrap items-center gap-1 overflow-x-auto scrollbar-none [&::-webkit-scrollbar]:hidden">
          <legend className="sr-only">Project owner filter</legend>
          {filterOptions.map((option) => (
            <Button
              aria-label={option.label}
              aria-pressed={filter === option.value}
              className={cn(
                "h-8 shrink-0 gap-1.5 rounded-full px-3 text-xs xl:px-3.5 xl:text-sm",
                filter === option.value && SELECTED_MENU_ITEM_CLASSES,
              )}
              key={option.value}
              onClick={() => onFilterChange(option.value)}
              size="sm"
              type="button"
              variant="ghost"
            >
              {option.compactLabel ? (
                <>
                  <span className="xl:hidden">{option.compactLabel}</span>
                  <span className="hidden xl:inline">{option.label}</span>
                </>
              ) : (
                option.label
              )}
            </Button>
          ))}
        </fieldset>
      </div>
    </div>
  );
}
