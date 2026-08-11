import type {
  ProjectsFilter,
  ProjectsRepositoryScope,
  ProjectsSort,
  ProjectsViewMode,
  ProjectsWorkItemScope,
} from "@/features/projects/lib/projectsViewHelpers";
import { ProjectsListScopeDropdown } from "@/features/projects/ui/ProjectsListScopeDropdown";
import { ProjectsViewModeToggle } from "@/features/projects/ui/ProjectsToolbar";
import { cn } from "@/shared/lib/cn";

const PROJECT_SCOPE_OPTIONS: Array<{
  label: string;
  value: ProjectsRepositoryScope;
}> = [
  { label: "All", value: "all" },
  { label: "Accessible", value: "accessible" },
  { label: "My Projects", value: "mine" },
  { label: "Local", value: "local" },
];
const REPOSITORY_SCOPE_OPTIONS: Array<{
  label: string;
  value: ProjectsRepositoryScope;
}> = [
  { label: "All", value: "all" },
  { label: "Accessible", value: "accessible" },
  { label: "My Repositories", value: "mine" },
  { label: "Local", value: "local" },
  { label: "Buzz-hosted", value: "buzz" },
  { label: "Linked", value: "linked" },
];
const PULL_REQUEST_SCOPE_OPTIONS: Array<{
  label: string;
  value: ProjectsWorkItemScope;
}> = [
  { label: "All", value: "all" },
  { label: "My Pull Requests", value: "mine" },
];
const ISSUE_SCOPE_OPTIONS: Array<{
  label: string;
  value: ProjectsWorkItemScope;
}> = [
  { label: "All", value: "all" },
  { label: "My Issues", value: "mine" },
];

type ProjectsListHeaderBarProps = {
  filter: ProjectsFilter;
  issueScope: ProjectsWorkItemScope;
  onIssueScopeChange: (scope: ProjectsWorkItemScope) => void;
  onPullRequestScopeChange: (scope: ProjectsWorkItemScope) => void;
  onRepositoryScopeChange: (scope: ProjectsRepositoryScope) => void;
  onSortChange: (sort: ProjectsSort) => void;
  onViewModeChange: (viewMode: ProjectsViewMode) => void;
  pullRequestScope: ProjectsWorkItemScope;
  repositoryScope: ProjectsRepositoryScope;
  sort: ProjectsSort;
  /**
   * "row" renders as the first row of the list table (no chrome of its own —
   * the surrounding container provides border and rounding); "bar" renders as
   * a standalone rounded bar with identical proportions for the card grid.
   */
  variant: "bar" | "row";
  viewMode: ProjectsViewMode;
};

/**
 * Header for the Projects lists: scope selector on the left, sort + view
 * toggle on the right.
 */
export function ProjectsListHeaderBar({
  filter,
  issueScope,
  onIssueScopeChange,
  onPullRequestScopeChange,
  onRepositoryScopeChange,
  onSortChange,
  onViewModeChange,
  pullRequestScope,
  repositoryScope,
  sort,
  variant,
  viewMode,
}: ProjectsListHeaderBarProps) {
  const scopeDropdown =
    filter === "prs" ? (
      <ProjectsListScopeDropdown
        label="Filter pull requests"
        onChange={onPullRequestScopeChange}
        options={PULL_REQUEST_SCOPE_OPTIONS}
        value={pullRequestScope}
      />
    ) : filter === "issues" ? (
      <ProjectsListScopeDropdown
        label="Filter issues"
        onChange={onIssueScopeChange}
        options={ISSUE_SCOPE_OPTIONS}
        value={issueScope}
      />
    ) : filter === "projects" ? (
      <ProjectsListScopeDropdown
        label="Filter projects"
        onChange={onRepositoryScopeChange}
        options={PROJECT_SCOPE_OPTIONS}
        value={repositoryScope}
      />
    ) : (
      <ProjectsListScopeDropdown
        label="Filter repositories"
        onChange={onRepositoryScopeChange}
        options={REPOSITORY_SCOPE_OPTIONS}
        value={repositoryScope}
      />
    );

  return (
    <div
      className={cn(
        "flex flex-wrap items-center justify-between gap-2 bg-muted/20 px-3 py-1.5",
        variant === "bar"
          ? "rounded-xl border border-border/60"
          : "border-b border-border/60",
      )}
      data-testid="projects-list-header"
    >
      {scopeDropdown}
      <div className="flex flex-wrap items-center gap-2">
        <label className="flex items-center gap-2 text-xs text-muted-foreground">
          <span className="sr-only">Sort projects</span>
          <select
            className="h-8 rounded-md bg-transparent px-2 text-xs text-foreground outline-hidden hover:bg-muted/50 focus:ring-1 focus:ring-ring"
            onChange={(event) =>
              onSortChange(event.target.value as ProjectsSort)
            }
            value={sort}
          >
            <option value="updated">Recent activity</option>
            <option value="created">Created date</option>
            <option value="name">Name</option>
          </select>
        </label>
        <ProjectsViewModeToggle
          onViewModeChange={onViewModeChange}
          viewMode={viewMode}
        />
      </div>
    </div>
  );
}
