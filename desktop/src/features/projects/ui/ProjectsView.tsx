import {
  Bot,
  CalendarDays,
  FolderGit2,
  GitBranch,
  GitFork,
  LayoutGrid,
  List,
  MessageSquare,
  Trash2,
  Users,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import {
  type Project,
  type ProjectActivitySummary,
  useDeleteProjectMutation,
  useProjectActivitySummariesQuery,
  useProjectsQuery,
} from "@/features/projects/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { topChromeInset } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type ProjectsViewMode = "grid" | "list";
type ProjectsFilter = "all" | "mine" | "agents" | "users";
type ProjectsSort = "updated" | "created" | "name";

const PROJECTS_VIEW_MODE_STORAGE_KEY = "buzz.projects.viewMode";
const PROJECTS_FILTER_STORAGE_KEY = "buzz.projects.filter";
const PROJECTS_SORT_STORAGE_KEY = "buzz.projects.sort";
const MANY_PROJECTS_THRESHOLD = 12;

function readStoredViewMode(): ProjectsViewMode | null {
  try {
    const value = globalThis.localStorage?.getItem(
      PROJECTS_VIEW_MODE_STORAGE_KEY,
    );
    return value === "grid" || value === "list" ? value : null;
  } catch {
    return null;
  }
}

function writeStoredViewMode(viewMode: ProjectsViewMode) {
  try {
    globalThis.localStorage?.setItem(PROJECTS_VIEW_MODE_STORAGE_KEY, viewMode);
  } catch {
    // Persistence is best-effort; the in-memory toggle still works.
  }
}

function readStoredFilter(): ProjectsFilter {
  try {
    const value = globalThis.localStorage?.getItem(PROJECTS_FILTER_STORAGE_KEY);
    return value === "mine" || value === "agents" || value === "users"
      ? value
      : "all";
  } catch {
    return "all";
  }
}

function writeStoredFilter(filter: ProjectsFilter) {
  try {
    globalThis.localStorage?.setItem(PROJECTS_FILTER_STORAGE_KEY, filter);
  } catch {
    // Persistence is best-effort; the in-memory toggle still works.
  }
}

function readStoredSort(): ProjectsSort {
  try {
    const value = globalThis.localStorage?.getItem(PROJECTS_SORT_STORAGE_KEY);
    return value === "created" || value === "name" ? value : "updated";
  } catch {
    return "updated";
  }
}

function writeStoredSort(sort: ProjectsSort) {
  try {
    globalThis.localStorage?.setItem(PROJECTS_SORT_STORAGE_KEY, sort);
  } catch {
    // Persistence is best-effort; the in-memory toggle still works.
  }
}

function pluralize(count: number, singular: string, plural = `${singular}s`) {
  return `${count} ${count === 1 ? singular : plural}`;
}

function formatCreatedDate(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

function projectPeople(
  project: Project,
  summary?: ProjectActivitySummary,
): string[] {
  return [
    ...new Set(
      [
        project.owner,
        ...project.contributors,
        ...(summary?.participantPubkeys ?? []),
      ].map(normalizePubkey),
    ),
  ];
}

function getCloneLabel(project: Project) {
  return project.cloneUrls[0] ?? "Internal git clone URL pending";
}

function getDiscussionLabel(project: Project) {
  return project.projectChannelId ? "Discussion linked" : "No discussion";
}

function getActivityLabel(summary: ProjectActivitySummary | undefined) {
  if (!summary || summary.activityCount === 0) {
    return "No activity yet";
  }

  return `${pluralize(summary.issueCount, "issue")} · ${pluralize(
    summary.activityCount,
    "event",
  )}`;
}

function getProjectUpdatedAt(
  project: Project,
  summary: ProjectActivitySummary | undefined,
) {
  return summary?.updatedAt ?? project.createdAt;
}

function isProjectMine(project: Project, currentPubkey: string | undefined) {
  if (!currentPubkey) return false;
  const normalizedCurrentPubkey = normalizePubkey(currentPubkey);
  return (
    normalizePubkey(project.owner) === normalizedCurrentPubkey ||
    project.contributors.some(
      (pubkey) => normalizePubkey(pubkey) === normalizedCurrentPubkey,
    )
  );
}

function projectHasAgent(
  project: Project,
  people: string[],
  profiles: UserProfileLookup | undefined,
) {
  const projectPubkeys = [project.owner, ...people];
  return projectPubkeys.some(
    (pubkey) => profiles?.[normalizePubkey(pubkey)]?.isAgent === true,
  );
}

function projectOwnerIsUser(
  project: Project,
  profiles: UserProfileLookup | undefined,
) {
  return profiles?.[normalizePubkey(project.owner)]?.isAgent !== true;
}

function WorkOwnerBadge({
  avatarUrl,
  isAgent,
  label,
}: {
  avatarUrl: string | null;
  isAgent: boolean;
  label: string;
}) {
  return (
    <span className="inline-flex max-w-full items-center gap-1.5 rounded-full border border-border/50 bg-muted/30 px-1.5 py-0.5 text-xs text-muted-foreground">
      <UserAvatar
        accent={isAgent}
        avatarUrl={avatarUrl}
        displayName={label}
        size="xs"
        testId="project-work-owner-avatar"
      />
      <span className="truncate">
        {isAgent ? "Agent" : "Work by"}: {label}
      </span>
    </span>
  );
}

function ProjectPeopleStack({
  pubkeys,
  profiles,
  workOwnerPubkey,
}: {
  pubkeys: string[];
  profiles?: UserProfileLookup;
  workOwnerPubkey: string;
}) {
  const visible = pubkeys.slice(0, 4);
  const remaining = pubkeys.length - visible.length;

  if (visible.length === 0) {
    return null;
  }

  return (
    <div className="flex items-center -space-x-1.5">
      {visible.map((pubkey) => {
        const profile = profiles?.[normalizePubkey(pubkey)];
        const label = resolveUserLabel({ pubkey, profiles });
        return (
          <UserAvatar
            accent={
              normalizePubkey(pubkey) === normalizePubkey(workOwnerPubkey)
            }
            avatarUrl={profile?.avatarUrl ?? null}
            className="ring-2 ring-card"
            displayName={label}
            key={pubkey}
            size="xs"
          />
        );
      })}
      {remaining > 0 ? (
        <span className="flex h-5 min-w-5 items-center justify-center rounded-full bg-muted px-1 text-3xs font-semibold text-muted-foreground ring-2 ring-card">
          +{remaining}
        </span>
      ) : null}
    </div>
  );
}

function StatusPill({ status }: { status: string }) {
  return (
    <span className="shrink-0 rounded-full bg-muted px-2 py-0.5 text-2xs font-medium uppercase tracking-wide text-muted-foreground">
      {status}
    </span>
  );
}

function MetadataItem({
  icon: Icon,
  children,
}: {
  icon: React.ComponentType<{ className?: string }>;
  children: React.ReactNode;
}) {
  return (
    <span className="flex min-w-0 items-center gap-1.5">
      <Icon className="h-3.5 w-3.5 shrink-0 text-muted-foreground/70" />
      <span className="min-w-0 truncate">{children}</span>
    </span>
  );
}

function ProjectsViewModeToggle({
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

function EmptyState() {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-3 px-4 py-16 text-center">
      <FolderGit2 className="h-10 w-10 text-muted-foreground/40" />
      <div className="space-y-1">
        <p className="text-sm font-medium text-foreground">No projects yet</p>
        <p className="text-sm text-muted-foreground">
          Projects published to this relay will appear here.
        </p>
      </div>
    </div>
  );
}

function EmptyFilteredState() {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-3 rounded-xl border border-dashed border-border/60 px-4 py-12 text-center">
      <FolderGit2 className="h-9 w-9 text-muted-foreground/40" />
      <div className="space-y-1">
        <p className="text-sm font-medium text-foreground">
          No matching projects
        </p>
        <p className="text-sm text-muted-foreground">
          Try another owner filter or sort mode.
        </p>
      </div>
    </div>
  );
}

function ProjectsToolbar({
  filter,
  onFilterChange,
  onSortChange,
  onViewModeChange,
  projectCount,
  sort,
  totalProjectCount,
  viewMode,
}: {
  filter: ProjectsFilter;
  onFilterChange: (filter: ProjectsFilter) => void;
  onSortChange: (sort: ProjectsSort) => void;
  onViewModeChange: (viewMode: ProjectsViewMode) => void;
  projectCount: number;
  sort: ProjectsSort;
  totalProjectCount: number;
  viewMode: ProjectsViewMode;
}) {
  const filterOptions: Array<{ label: string; value: ProjectsFilter }> = [
    { label: "All", value: "all" },
    { label: "Mine", value: "mine" },
    { label: "Agents", value: "agents" },
    { label: "Users", value: "users" },
  ];

  return (
    <div className="mb-4 flex flex-col gap-3 border-b border-border/50 pb-4">
      <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-lg font-semibold text-foreground">Projects</h2>
            <span className="rounded-full bg-muted px-2 py-0.5 text-2xs font-medium uppercase tracking-wide text-muted-foreground">
              {pluralize(projectCount, "project")}
              {projectCount !== totalProjectCount
                ? ` of ${totalProjectCount}`
                : ""}
            </span>
          </div>
          <p className="max-w-2xl text-sm text-muted-foreground">
            Internal git projects bring code, issues, discussion, and agent work
            into one shared space.
          </p>
        </div>
        <ProjectsViewModeToggle
          onViewModeChange={onViewModeChange}
          viewMode={viewMode}
        />
      </div>

      <div className="flex flex-col gap-2 lg:flex-row lg:items-center lg:justify-between">
        <fieldset className="flex flex-wrap items-center gap-1 rounded-lg border border-border/60 bg-muted/30 p-1">
          <legend className="sr-only">Project owner filter</legend>
          {filterOptions.map((option) => (
            <Button
              aria-pressed={filter === option.value}
              className="h-7 gap-1.5 px-2"
              key={option.value}
              onClick={() => onFilterChange(option.value)}
              size="xs"
              type="button"
              variant={filter === option.value ? "secondary" : "ghost"}
            >
              {option.value === "agents" ? (
                <Bot className="h-3.5 w-3.5" />
              ) : null}
              {option.value === "users" ? (
                <Users className="h-3.5 w-3.5" />
              ) : null}
              {option.label}
            </Button>
          ))}
        </fieldset>

        <label className="flex items-center gap-2 text-xs text-muted-foreground">
          Sort
          <select
            className="h-7 rounded-md border border-border/60 bg-background px-2 text-xs text-foreground outline-hidden focus:ring-1 focus:ring-ring"
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
      </div>
    </div>
  );
}

function ProjectCardButton({
  project,
  onOpen,
}: {
  project: Project;
  onOpen: (project: Project) => void;
}) {
  return (
    <button
      className="absolute inset-0 rounded-xl"
      onClick={() => onOpen(project)}
      type="button"
    >
      <span className="sr-only">View {project.name}</span>
    </button>
  );
}

function ProjectDeleteButton({
  project,
  disabled,
  onDelete,
}: {
  project: Project;
  disabled: boolean;
  onDelete: (project: Project) => void;
}) {
  return (
    <Button
      aria-label={`Delete ${project.name}`}
      className="h-7 w-7 text-muted-foreground opacity-0 transition-opacity hover:text-destructive group-hover:opacity-100"
      disabled={disabled}
      onClick={(event) => {
        event.stopPropagation();
        onDelete(project);
      }}
      size="icon"
      type="button"
      variant="ghost"
    >
      <Trash2 className="h-3.5 w-3.5" />
    </Button>
  );
}

function ProjectGridCard({
  project,
  people,
  profiles,
  summary,
  onDelete,
  onOpen,
  deleteDisabled,
}: {
  project: Project;
  people: string[];
  profiles?: UserProfileLookup;
  summary: ProjectActivitySummary | undefined;
  onDelete: (project: Project) => void;
  onOpen: (project: Project) => void;
  deleteDisabled: boolean;
}) {
  const ownerProfile = profiles?.[normalizePubkey(project.owner)];
  const ownerLabel = resolveUserLabel({ pubkey: project.owner, profiles });

  return (
    <Card
      className="group relative flex min-h-52 flex-col overflow-hidden border-border/50 bg-card/60 p-3 shadow-none transition-colors hover:border-border hover:bg-muted/30"
      data-testid={`project-card-${project.dtag}`}
    >
      <ProjectCardButton onOpen={onOpen} project={project} />
      <div className="flex min-h-0 flex-1 flex-col gap-3">
        <div className="flex min-w-0 items-start justify-between gap-3">
          <div className="min-w-0 space-y-1">
            <div className="flex min-w-0 items-center gap-2">
              <FolderGit2 className="h-4 w-4 shrink-0 text-muted-foreground" />
              <span className="truncate text-sm font-medium text-foreground">
                {project.name}
              </span>
            </div>
            <p className="truncate font-mono text-2xs text-muted-foreground/70">
              {project.dtag}
            </p>
          </div>
          <StatusPill status={project.status} />
        </div>

        <WorkOwnerBadge
          avatarUrl={ownerProfile?.avatarUrl ?? null}
          isAgent={ownerProfile?.isAgent === true}
          label={ownerLabel}
        />

        <p className="line-clamp-3 min-h-12 text-sm text-muted-foreground">
          {project.description || "A shared space for internal git work."}
        </p>

        <div className="grid grid-cols-2 gap-2 text-xs text-muted-foreground">
          <MetadataItem icon={GitBranch}>{project.defaultBranch}</MetadataItem>
          <MetadataItem icon={Users}>
            {pluralize(people.length, "person", "people")}
          </MetadataItem>
          <MetadataItem icon={MessageSquare}>
            {getDiscussionLabel(project)}
          </MetadataItem>
          <MetadataItem icon={CalendarDays}>
            {formatCreatedDate(project.createdAt)}
          </MetadataItem>
        </div>

        <div className="mt-auto space-y-2 rounded-lg border border-border/50 bg-muted/25 px-2.5 py-2">
          <div className="flex min-w-0 items-center justify-between gap-2">
            <p className="truncate text-xs text-muted-foreground">
              {getActivityLabel(summary)}
            </p>
            <div className="relative z-10 flex shrink-0 items-center gap-1">
              <ProjectPeopleStack
                profiles={profiles}
                pubkeys={people}
                workOwnerPubkey={project.owner}
              />
              <ProjectDeleteButton
                disabled={deleteDisabled}
                onDelete={onDelete}
                project={project}
              />
            </div>
          </div>
          <div className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground/80">
            <GitFork className="h-3.5 w-3.5 shrink-0" />
            <span className="truncate font-mono">{getCloneLabel(project)}</span>
          </div>
        </div>
      </div>
    </Card>
  );
}

function ProjectListRow({
  project,
  people,
  profiles,
  summary,
  onDelete,
  onOpen,
  deleteDisabled,
}: {
  project: Project;
  people: string[];
  profiles?: UserProfileLookup;
  summary: ProjectActivitySummary | undefined;
  onDelete: (project: Project) => void;
  onOpen: (project: Project) => void;
  deleteDisabled: boolean;
}) {
  const ownerProfile = profiles?.[normalizePubkey(project.owner)];
  const ownerLabel = resolveUserLabel({ pubkey: project.owner, profiles });

  return (
    <Card
      className="group relative overflow-hidden border-border/50 bg-card/60 p-3 shadow-none transition-colors hover:border-border hover:bg-muted/30"
      data-testid={`project-row-${project.dtag}`}
    >
      <ProjectCardButton onOpen={onOpen} project={project} />
      <div className="grid gap-3 lg:grid-cols-[minmax(0,1.5fr)_minmax(14rem,1fr)_auto] lg:items-center">
        <div className="min-w-0 space-y-1">
          <div className="flex min-w-0 items-center gap-2">
            <FolderGit2 className="h-4 w-4 shrink-0 text-muted-foreground" />
            <span className="truncate text-sm font-medium text-foreground">
              {project.name}
            </span>
            <StatusPill status={project.status} />
          </div>
          <p className="line-clamp-1 text-sm text-muted-foreground">
            {project.description || "A shared space for internal git work."}
          </p>
          <WorkOwnerBadge
            avatarUrl={ownerProfile?.avatarUrl ?? null}
            isAgent={ownerProfile?.isAgent === true}
            label={ownerLabel}
          />
        </div>

        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
            <MetadataItem icon={GitBranch}>
              {project.defaultBranch}
            </MetadataItem>
            <MetadataItem icon={Users}>
              {pluralize(people.length, "person", "people")}
            </MetadataItem>
            <MetadataItem icon={MessageSquare}>
              {getDiscussionLabel(project)}
            </MetadataItem>
            <MetadataItem icon={CalendarDays}>
              {formatCreatedDate(project.createdAt)}
            </MetadataItem>
          </div>
          <div className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground/75">
            <GitFork className="h-3.5 w-3.5 shrink-0" />
            <span className="truncate font-mono">{getCloneLabel(project)}</span>
          </div>
        </div>

        <div className="relative z-10 flex min-w-0 items-center justify-start gap-2 lg:justify-end">
          <p className="truncate text-xs text-muted-foreground">
            {getActivityLabel(summary)}
          </p>
          <ProjectPeopleStack
            profiles={profiles}
            pubkeys={people}
            workOwnerPubkey={project.owner}
          />
          <ProjectDeleteButton
            disabled={deleteDisabled}
            onDelete={onDelete}
            project={project}
          />
        </div>
      </div>
    </Card>
  );
}

export function ProjectsView() {
  const { goProject } = useAppNavigation();
  const projectsQuery = useProjectsQuery();
  const identityQuery = useIdentityQuery();
  const projects = projectsQuery.data ?? [];
  const activitySummariesQuery = useProjectActivitySummariesQuery(projects);
  const [storedViewMode, setStoredViewMode] =
    React.useState<ProjectsViewMode | null>(() => readStoredViewMode());
  const [filter, setFilter] = React.useState<ProjectsFilter>(() =>
    readStoredFilter(),
  );
  const [sort, setSort] = React.useState<ProjectsSort>(() => readStoredSort());
  const viewMode =
    storedViewMode ??
    (projects.length > MANY_PROJECTS_THRESHOLD ? "list" : "grid");

  const projectPubkeys = React.useMemo(
    () => [
      ...new Set(
        projects.flatMap((project) =>
          projectPeople(
            project,
            activitySummariesQuery.data?.[project.repoAddress],
          ),
        ),
      ),
    ],
    [activitySummariesQuery.data, projects],
  );
  const profilesQuery = useUsersBatchQuery(projectPubkeys, {
    enabled: projectPubkeys.length > 0,
  });
  const profiles = profilesQuery.data?.profiles;
  const deleteProjectMutation = useDeleteProjectMutation();

  const handleViewModeChange = React.useCallback(
    (nextViewMode: ProjectsViewMode) => {
      setStoredViewMode(nextViewMode);
      writeStoredViewMode(nextViewMode);
    },
    [],
  );

  const handleFilterChange = React.useCallback((nextFilter: ProjectsFilter) => {
    setFilter(nextFilter);
    writeStoredFilter(nextFilter);
  }, []);

  const handleSortChange = React.useCallback((nextSort: ProjectsSort) => {
    setSort(nextSort);
    writeStoredSort(nextSort);
  }, []);

  const visibleProjects = React.useMemo(() => {
    const currentPubkey = identityQuery.data?.pubkey;
    return projects
      .filter((project) => {
        const summary = activitySummariesQuery.data?.[project.repoAddress];
        const people = projectPeople(project, summary);
        if (filter === "mine") return isProjectMine(project, currentPubkey);
        if (filter === "agents")
          return projectHasAgent(project, people, profiles);
        if (filter === "users") return projectOwnerIsUser(project, profiles);
        return true;
      })
      .sort((left, right) => {
        const leftSummary = activitySummariesQuery.data?.[left.repoAddress];
        const rightSummary = activitySummariesQuery.data?.[right.repoAddress];
        if (sort === "name") {
          return left.name.localeCompare(right.name);
        }
        if (sort === "created") {
          return right.createdAt - left.createdAt;
        }
        return (
          getProjectUpdatedAt(right, rightSummary) -
          getProjectUpdatedAt(left, leftSummary)
        );
      });
  }, [
    activitySummariesQuery.data,
    filter,
    identityQuery.data?.pubkey,
    profiles,
    projects,
    sort,
  ]);

  const handleOpenProject = React.useCallback(
    (project: Project) => {
      void goProject(project.dtag);
    },
    [goProject],
  );

  const handleDeleteProject = React.useCallback(
    async (project: Project) => {
      const confirmed = window.confirm(`Delete ${project.name}?`);
      if (!confirmed) return;

      try {
        await deleteProjectMutation.mutateAsync(project);
        toast.success("Project card deleted");
      } catch (error) {
        toast.error(
          error instanceof Error
            ? error.message
            : "Failed to delete project card",
        );
      }
    },
    [deleteProjectMutation],
  );

  if (projectsQuery.isLoading) {
    return null;
  }

  if (projectsQuery.isError) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-2 text-muted-foreground">
        <p className="text-sm text-red-400">Failed to load projects</p>
        <Button
          onClick={() => void projectsQuery.refetch()}
          size="sm"
          variant="outline"
        >
          Retry
        </Button>
      </div>
    );
  }

  if (projects.length === 0) {
    return <EmptyState />;
  }

  return (
    <div
      className={cn(
        "flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto px-4 pb-4",
        topChromeInset.padding,
      )}
    >
      <ProjectsToolbar
        filter={filter}
        onFilterChange={handleFilterChange}
        onSortChange={handleSortChange}
        onViewModeChange={handleViewModeChange}
        projectCount={visibleProjects.length}
        sort={sort}
        totalProjectCount={projects.length}
        viewMode={viewMode}
      />

      {visibleProjects.length === 0 ? (
        <EmptyFilteredState />
      ) : viewMode === "grid" ? (
        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          {visibleProjects.map((project) => {
            const summary = activitySummariesQuery.data?.[project.repoAddress];
            return (
              <ProjectGridCard
                deleteDisabled={deleteProjectMutation.isPending}
                key={project.id}
                onDelete={(nextProject) =>
                  void handleDeleteProject(nextProject)
                }
                onOpen={handleOpenProject}
                people={projectPeople(project, summary)}
                profiles={profiles}
                project={project}
                summary={summary}
              />
            );
          })}
        </div>
      ) : (
        <div className="space-y-2">
          {visibleProjects.map((project) => {
            const summary = activitySummariesQuery.data?.[project.repoAddress];
            return (
              <ProjectListRow
                deleteDisabled={deleteProjectMutation.isPending}
                key={project.id}
                onDelete={(nextProject) =>
                  void handleDeleteProject(nextProject)
                }
                onOpen={handleOpenProject}
                people={projectPeople(project, summary)}
                profiles={profiles}
                project={project}
                summary={summary}
              />
            );
          })}
        </div>
      )}
    </div>
  );
}
