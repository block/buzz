import type { ReactNode } from "react";
import { useProjectSelection } from "../lib/useProjectSelection";
import { Plus, Search } from "lucide-react";
import type { ProjectsFilter, ProjectsSort } from "../lib/projectsViewHelpers";
import { ProjectsSortSelect } from "./ProjectsListHeaderBar";
import { projectsSectionTitle } from "./projectsSectionMeta";
import { Button } from "@/shared/ui/button";

/** Content controls for Projects when navigation lives in a separate rail. */
export function ProjectsPanelToolbar({
  filter,
  query,
  onQueryChange,
  sort,
  onSortChange,
  onCreate,
  canCreateTarget,
}: {
  filter: ProjectsFilter;
  query: string;
  onQueryChange: (query: string) => void;
  sort: ProjectsSort;
  onSortChange: (sort: ProjectsSort) => void;
  onCreate: () => void;
  canCreateTarget: boolean;
}) {
  const label =
    filter === "repositories"
      ? "Add repository"
      : filter === "channels"
        ? "Add channel"
        : filter === "issues"
          ? "Create task"
          : filter === "prs"
            ? "Create review"
            : "Create project";
  return (
    <div className="flex min-w-0 flex-1 items-center gap-2">
      <label className="flex min-w-0 flex-1 items-center gap-2">
        <Search
          aria-hidden="true"
          className="h-4 w-4 shrink-0 text-muted-foreground"
        />
        <input
          type="search"
          aria-label={`Search ${projectsSectionTitle(filter)}`}
          placeholder={`Search ${projectsSectionTitle(filter).toLowerCase()}`}
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
          className="h-9 min-w-0 w-full bg-transparent text-sm outline-none"
        />
      </label>
      {filter !== "all" && filter !== "channels" && (
        <ProjectsSortSelect sort={sort} onChange={onSortChange} />
      )}
      <Button
        variant="ghost"
        size="icon"
        className="h-8 w-8 shrink-0"
        aria-label={label}
        title={label}
        onClick={onCreate}
        disabled={
          (filter === "repositories" || filter === "channels") &&
          !canCreateTarget
        }
      >
        <Plus aria-hidden="true" className="h-4 w-4" />
      </Button>
    </div>
  );
}

/** Keep selection actions available without the redundant context rail. */
export function ProjectsPanelSelection({ children }: { children: ReactNode }) {
  const selection = useProjectSelection();
  return selection?.items.length ? children : null;
}
