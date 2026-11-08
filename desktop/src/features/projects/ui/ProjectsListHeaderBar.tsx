import type {
  ProjectsSort,
  ProjectsViewMode,
} from "@/features/projects/lib/projectsViewHelpers";
import { ProjectsViewModeToggle } from "@/features/projects/ui/ProjectsToolbar";

type ProjectsListHeaderBarProps = {
  onViewModeChange: (viewMode: ProjectsViewMode) => void;
  viewMode: ProjectsViewMode;
};

/** Shared Projects sort control used by the top navigation/search row. */
export function ProjectsSortSelect({
  onChange,
  sort,
}: {
  onChange: (sort: ProjectsSort) => void;
  sort: ProjectsSort;
}) {
  return (
    <label className="flex items-center gap-2 text-xs text-muted-foreground">
      <span className="sr-only">Sort projects</span>
      <select
        className="h-8 rounded-md bg-transparent px-2 text-xs text-foreground outline-hidden hover:bg-muted/50 focus:ring-1 focus:ring-ring"
        onChange={(event) => onChange(event.target.value as ProjectsSort)}
        value={sort}
      >
        <option value="updated">Recent activity</option>
        <option value="created">Created date</option>
        <option value="name">Name</option>
      </select>
    </label>
  );
}

/** Compact layout controls rendered in the Projects section header. */
export function ProjectsListHeaderBar({
  onViewModeChange,
  viewMode,
}: ProjectsListHeaderBarProps) {
  return (
    <div
      className="ml-auto flex flex-wrap items-center justify-end gap-2"
      data-testid="projects-list-header"
    >
      <ProjectsViewModeToggle
        onViewModeChange={onViewModeChange}
        viewMode={viewMode}
      />
    </div>
  );
}
