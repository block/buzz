import {
  CalendarDays,
  FolderGit2,
  GitFork,
  LayoutGrid,
  List,
  MessageSquare,
  MoreHorizontal,
  Plus,
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
  useProjectLocalRepositoriesQuery,
  useProjectsQuery,
} from "@/features/projects/hooks";
import { getDiscussionLabel } from "@/features/projects/lib/projectLabels";
import { hasLocalCheckout } from "@/features/projects/lib/projectLocalRepos";
import { useWorkspaces } from "@/features/workspaces/useWorkspaces";
import { useIdentityQuery } from "@/shared/api/hooks";
import { ProjectsOverviewPanel } from "@/features/projects/ui/ProjectsOverviewPanel";
import { useMainInsetRef } from "@/shared/layout/MainInsetContext";
import {
  channelChrome,
  channelContentTopPaddingMeasurement,
  topChromeInset,
} from "@/shared/layout/chromeLayout";
import { useMeasuredCssVariable } from "@/shared/layout/useMeasuredCssVariable";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type ProjectsViewMode = "grid" | "list";
type ProjectsFilter =
  | "all"
  | "mine"
  | "local"
  | "repositories"
  | "prs"
  | "agents"
  | "users";
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
    return value === "mine" ||
      value === "local" ||
      value === "repositories" ||
      value === "prs" ||
      value === "agents" ||
      value === "users"
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

function normalizeRepositoryUrl(url: string) {
  try {
    const parsed = new URL(url);
    const normalizedPath = parsed.pathname
      .replace(/\/+$/, "")
      .replace(/\.git$/i, "")
      .toLowerCase();
    return `${parsed.hostname.toLowerCase()}${normalizedPath}`;
  } catch {
    return url
      .trim()
      .replace(/\/+$/, "")
      .replace(/\.git$/i, "")
      .toLowerCase();
  }
}

function getClonePathLabel(project: Project) {
  const cloneUrl = project.cloneUrls[0];
  if (!cloneUrl) return "Clone path pending";

  try {
    const parsed = new URL(cloneUrl);
    return `${parsed.hostname}${parsed.pathname}`;
  } catch {
    return cloneUrl;
  }
}

function repositoryIdentityKey(project: Project) {
  const cloneUrl = project.cloneUrls[0];
  if (cloneUrl) return normalizeRepositoryUrl(cloneUrl);
  return (project.name || project.dtag).trim().toLowerCase();
}

function uniqueRepositories(projects: Project[]) {
  const seen = new Set<string>();
  return projects.filter((project) => {
    const key = repositoryIdentityKey(project);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function getActivityLabel(summary: ProjectActivitySummary | undefined) {
  if (!summary || summary.activityCount === 0) {
    return "No activity yet";
  }

  const parts = [pluralize(summary.issueCount, "issue")];
  if (summary.prCount > 0) {
    parts.push(pluralize(summary.prCount, "PR"));
  }
  parts.push(pluralize(summary.activityCount, "event"));
  return parts.join(" · ");
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
function isProjectOwnedByCurrentUser(
  project: Project,
  currentPubkey: string | undefined,
) {
  return currentPubkey
    ? normalizePubkey(project.owner) === normalizePubkey(currentPubkey)
    : false;
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
  if (status === "active") {
    return null;
  }

  return (
    <span className="shrink-0 rounded-full border border-border/60 bg-background/80 px-2 py-0.5 text-2xs font-medium uppercase tracking-wide text-muted-foreground shadow-xs">
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
  onCreateProject,
  onFilterChange,
  onSortChange,
  onViewModeChange,
  sort,
  viewMode,
}: {
  filter: ProjectsFilter;
  onCreateProject: () => void;
  onFilterChange: (filter: ProjectsFilter) => void;
  onSortChange: (sort: ProjectsSort) => void;
  onViewModeChange: (viewMode: ProjectsViewMode) => void;
  sort: ProjectsSort;
  viewMode: ProjectsViewMode;
}) {
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

      <div className="flex flex-col gap-2 lg:flex-row lg:items-center lg:justify-between">
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

        <div className="flex flex-wrap items-center gap-2 lg:justify-end">
          <label className="flex items-center gap-2 text-xs text-muted-foreground">
            <span className="sr-only">Sort projects</span>
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
          <ProjectsViewModeToggle
            onViewModeChange={onViewModeChange}
            viewMode={viewMode}
          />
        </div>
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

function ProjectActionsMenu({
  project,
  canDelete,
  disabled,
  onDelete,
}: {
  project: Project;
  canDelete: boolean;
  disabled: boolean;
  onDelete: (project: Project) => Promise<void> | void;
}) {
  const [confirmOpen, setConfirmOpen] = React.useState(false);

  return (
    <AlertDialog onOpenChange={setConfirmOpen} open={confirmOpen}>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <Button
            aria-label={`More options for ${project.name}`}
            className="relative z-20 h-7 w-7 text-muted-foreground hover:text-foreground"
            onClick={(event) => event.stopPropagation()}
            size="icon"
            type="button"
            variant="ghost"
          >
            <MoreHorizontal className="h-4 w-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="min-w-48">
          <DropdownMenuItem
            className="text-destructive focus:text-destructive"
            disabled={!canDelete || disabled}
            onSelect={(event) => {
              event.preventDefault();
              event.stopPropagation();
              if (canDelete && !disabled) {
                setConfirmOpen(true);
              }
            }}
          >
            <Trash2 className="h-4 w-4" />
            Delete project
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <AlertDialogContent
        data-testid={`project-delete-confirm-${project.dtag}`}
      >
        <AlertDialogHeader>
          <AlertDialogTitle>Delete project?</AlertDialogTitle>
          <AlertDialogDescription>
            Delete {project.name} from Projects for everyone. This can only be
            done for projects you own and cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button disabled={disabled} type="button" variant="outline">
              Cancel
            </Button>
          </AlertDialogCancel>
          <AlertDialogAction asChild>
            <Button
              data-testid={`project-delete-confirm-button-${project.dtag}`}
              disabled={disabled}
              onClick={(event) => {
                event.preventDefault();
                void Promise.resolve(onDelete(project)).then(() =>
                  setConfirmOpen(false),
                );
              }}
              type="button"
              variant="destructive"
            >
              {disabled ? "Deleting..." : "Delete project"}
            </Button>
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function ProjectGridCard({
  project,
  people,
  profiles,
  summary,
  canDelete,
  deleteDisabled,
  onDelete,
  onOpen,
}: {
  project: Project;
  people: string[];
  profiles?: UserProfileLookup;
  summary: ProjectActivitySummary | undefined;
  canDelete: boolean;
  deleteDisabled: boolean;
  onDelete: (project: Project) => Promise<void> | void;
  onOpen: (project: Project) => void;
}) {
  return (
    <Card
      className="group relative flex min-h-48 flex-col overflow-hidden border-border/60 bg-gradient-to-br from-card via-card to-muted/30 p-4 shadow-none transition-colors duration-150 hover:border-primary/30"
      data-testid={`project-card-${project.dtag}`}
    >
      <ProjectCardButton onOpen={onOpen} project={project} />
      <div className="flex min-h-0 flex-1 flex-col gap-3">
        <div className="flex min-w-0 items-start justify-between gap-3">
          <div className="min-w-0 space-y-1.5">
            <div className="flex min-w-0 items-center gap-2">
              <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary shadow-inner">
                <FolderGit2 className="h-4 w-4" />
              </span>
              <div className="min-w-0 space-y-1">
                <span className="block truncate text-sm font-semibold text-foreground">
                  {project.name}
                </span>
                <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                  <MetadataItem icon={Users}>
                    {pluralize(people.length, "person", "people")}
                  </MetadataItem>
                  <MetadataItem icon={CalendarDays}>
                    {formatCreatedDate(project.createdAt)}
                  </MetadataItem>
                </div>
              </div>
            </div>
          </div>
          <div className="relative z-10 flex items-center gap-1">
            <StatusPill status={project.status} />
            <ProjectActionsMenu
              canDelete={canDelete}
              disabled={deleteDisabled}
              onDelete={onDelete}
              project={project}
            />
          </div>
        </div>

        <p className="line-clamp-2 min-h-10 text-sm text-muted-foreground">
          {project.description || "A shared space for internal git work."}
        </p>

        <div className="flex min-w-0 items-center gap-1.5 rounded-lg bg-muted/45 px-2.5 py-1.5 text-xs text-muted-foreground/80">
          <GitFork className="h-3.5 w-3.5 shrink-0" />
          <span className="truncate font-mono">
            {getClonePathLabel(project)}
          </span>
        </div>

        <div className="mt-auto space-y-2 rounded-lg bg-muted/70 px-2.5 py-2">
          <div className="flex min-w-0 items-center justify-between gap-2">
            <p className="truncate text-xs font-medium text-muted-foreground">
              {getActivityLabel(summary)}
            </p>
            <div className="relative z-10 flex shrink-0 items-center gap-1">
              <ProjectPeopleStack
                profiles={profiles}
                pubkeys={people}
                workOwnerPubkey={project.owner}
              />
            </div>
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
  canDelete,
  deleteDisabled,
  onDelete,
  onOpen,
}: {
  project: Project;
  people: string[];
  profiles?: UserProfileLookup;
  summary: ProjectActivitySummary | undefined;
  canDelete: boolean;
  deleteDisabled: boolean;
  onDelete: (project: Project) => Promise<void> | void;
  onOpen: (project: Project) => void;
}) {
  return (
    <Card
      className="group relative overflow-hidden border-border/60 bg-gradient-to-r from-card via-card to-muted/20 p-3 shadow-none transition-colors duration-150 hover:border-primary/30"
      data-testid={`project-row-${project.dtag}`}
    >
      <ProjectCardButton onOpen={onOpen} project={project} />
      <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_17rem_16rem] lg:items-center">
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
          <div className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground/75">
            <GitFork className="h-3.5 w-3.5 shrink-0" />
            <span className="truncate font-mono">
              {getClonePathLabel(project)}
            </span>
          </div>
        </div>

        <div className="min-w-0 justify-self-end space-y-1 lg:w-68">
          <div className="flex flex-wrap items-center justify-start gap-x-3 gap-y-1 text-left text-xs text-muted-foreground">
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
        </div>

        <div className="relative z-10 flex min-w-0 items-center justify-start gap-2 rounded-lg bg-muted/70 px-2.5 py-2 lg:w-full lg:justify-between">
          <p className="truncate text-xs font-medium text-muted-foreground">
            {getActivityLabel(summary)}
          </p>
          <ProjectPeopleStack
            profiles={profiles}
            pubkeys={people}
            workOwnerPubkey={project.owner}
          />
          <ProjectActionsMenu
            canDelete={canDelete}
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
  const { activeWorkspace } = useWorkspaces();
  const mainInsetRef = useMainInsetRef();
  const projectsHeaderChromeRef = useMeasuredCssVariable({
    targetRef: mainInsetRef,
    ...channelContentTopPaddingMeasurement,
  });
  const projectsQuery = useProjectsQuery();
  const identityQuery = useIdentityQuery();
  const projects = projectsQuery.data ?? [];
  const activitySummariesQuery = useProjectActivitySummariesQuery(projects);
  const localRepositoriesQuery = useProjectLocalRepositoriesQuery(
    activeWorkspace?.reposDir,
  );
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
  const currentPubkey = identityQuery.data?.pubkey;

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
  const handleCreateProject = React.useCallback(() => {
    toast.info("Project creation is not available yet.");
  }, []);

  const visibleProjects = React.useMemo(() => {
    const localRepoNames = new Set(
      (localRepositoriesQuery.data ?? []).map((repository) => repository.name),
    );
    const sortedProjects = projects
      .filter((project) => {
        const summary = activitySummariesQuery.data?.[project.repoAddress];
        const people = projectPeople(project, summary);
        if (filter === "mine") return isProjectMine(project, currentPubkey);
        if (filter === "local")
          return hasLocalCheckout(project, localRepoNames);
        if (filter === "prs") return (summary?.prCount ?? 0) > 0;
        if (filter === "agents") {
          return projectHasAgent(project, people, profiles);
        }
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

    return filter === "repositories"
      ? uniqueRepositories(sortedProjects)
      : sortedProjects;
  }, [
    activitySummariesQuery.data,
    currentPubkey,
    filter,
    localRepositoriesQuery.data,
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
      try {
        await deleteProjectMutation.mutateAsync(project);
        toast.success("Project deleted");
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to delete project",
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
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <div
        className={cn(
          "pointer-events-none relative z-30 overflow-hidden rounded-tl-xl bg-background/80 backdrop-blur-md supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55",
          channelChrome.negativeMargin,
          topChromeInset.divider,
        )}
        ref={projectsHeaderChromeRef}
      >
        <ProjectsToolbar
          filter={filter}
          onCreateProject={handleCreateProject}
          onFilterChange={handleFilterChange}
          onSortChange={handleSortChange}
          onViewModeChange={handleViewModeChange}
          sort={sort}
          viewMode={viewMode}
        />
      </div>

      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto px-4 pb-4">
        <div className="pt-[calc(var(--buzz-channel-content-top-padding,5.75rem)_+_1rem)]">
          {filter === "all" ? (
            <ProjectsOverviewPanel
              localRepositoryCount={localRepositoriesQuery.data?.length ?? 0}
              onOpenProject={handleOpenProject}
              profiles={profiles}
              projects={projects}
              relayName={activeWorkspace?.name || "Relay"}
              summaries={activitySummariesQuery.data}
            />
          ) : null}
          {visibleProjects.length === 0 ? (
            <EmptyFilteredState />
          ) : viewMode === "grid" ? (
            <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
              {visibleProjects.map((project) => {
                const summary =
                  activitySummariesQuery.data?.[project.repoAddress];
                return (
                  <ProjectGridCard
                    canDelete={isProjectOwnedByCurrentUser(
                      project,
                      currentPubkey,
                    )}
                    deleteDisabled={deleteProjectMutation.isPending}
                    key={project.id}
                    onDelete={handleDeleteProject}
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
                const summary =
                  activitySummariesQuery.data?.[project.repoAddress];
                return (
                  <ProjectListRow
                    canDelete={isProjectOwnedByCurrentUser(
                      project,
                      currentPubkey,
                    )}
                    deleteDisabled={deleteProjectMutation.isPending}
                    key={project.id}
                    onDelete={handleDeleteProject}
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
      </div>
    </div>
  );
}
