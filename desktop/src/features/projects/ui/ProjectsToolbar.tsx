import { LayoutGrid, List, Plus } from "lucide-react";

import { Button } from "@/shared/ui/button";

type ProjectsViewMode = "grid" | "list";
type ProjectsFilter =
  | "all"
  | "mine"
  | "local"
  | "repositories"
  | "prs"
  | "agents"
  | "users";

type ProjectsToolbarProps = {
  filter: ProjectsFilter;
  onCreateProject: () => void;
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
    <fieldset className="flex items-center rounded-lg border border-border/60 bg-muted/30 p-1">
      <legend className="sr-only">Project layout</legend>
      <Button
        aria-pressed={viewMode === "grid"}
        className="h-7 gap-1.5 px-2"
        onClick={() => onViewModeChange("grid")}
        size="xs"
        type="button"
        variant={viewMode === "grid" ? "secondary" : "ghost"}
      >
        <LayoutGrid className="h-3.5 w-3.5" />
        Grid
      </Button>
      <Button
        aria-pressed={viewMode === "list"}
        className="h-7 gap-1.5 px-2"
        onClick={() => onViewModeChange("list")}
        size="xs"
        type="button"
        variant={viewMode === "list" ? "secondary" : "ghost"}
      >
        <List className="h-3.5 w-3.5" />
        List
      </Button>
    </fieldset>
  );
}

export function ProjectsToolbar({
  filter,
  onCreateProject,
  onFilterChange,
}: ProjectsToolbarProps) {
  const filterOptions: Array<{ label: string; value: ProjectsFilter }> = [
    { label: "All", value: "all" },
    { label: "Mine", value: "mine" },
    { label: "Local", value: "local" },
    { label: "Repositories", value: "repositories" },
    { label: "PRs", value: "prs" },
    { label: "Agents", value: "agents" },
    { label: "Users", value: "users" },
  ];

  return (
    <div
      className="pointer-events-auto flex flex-col gap-3 px-5 py-2"
      data-tauri-drag-region
    >
      <div className="flex min-h-9 flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
        <h2 className="min-w-0 text-lg font-semibold text-foreground">
          Projects
        </h2>
        <Button
          className="h-8 gap-1.5 self-start rounded-full border-border/60 bg-background/70 px-3 text-muted-foreground shadow-none hover:bg-muted/60 hover:text-foreground lg:self-auto"
          data-testid="create-project-button"
          onClick={onCreateProject}
          size="sm"
          type="button"
          variant="outline"
        >
          <Plus className="h-3.5 w-3.5" />
          Create Project
        </Button>
      </div>

      <fieldset className="flex flex-wrap items-center gap-0.5">
        <legend className="sr-only">Project owner filter</legend>
        {filterOptions.map((option) => (
          <Button
            aria-pressed={filter === option.value}
            className="h-8 gap-1.5 rounded-full px-3"
            key={option.value}
            onClick={() => onFilterChange(option.value)}
            size="sm"
            type="button"
            variant={filter === option.value ? "secondary" : "ghost"}
          >
            {option.label}
          </Button>
        ))}
      </fieldset>
    </div>
  );
}
