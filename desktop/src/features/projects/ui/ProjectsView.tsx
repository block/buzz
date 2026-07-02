import {
  CalendarDays,
  FolderGit2,
  GitCommit,
  GitFork,
  MoreHorizontal,
  TerminalSquare,
  Trash2,
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
  type ProjectIssue,
  type ProjectPullRequest,
  useDeleteProjectMutation,
  useProjectActivitySummariesQuery,
  useProjectLocalRepositoriesQuery,
  useProjectsIssuesQuery,
  useProjectsPullRequestsQuery,
  useProjectsQuery,
} from "@/features/projects/hooks";
import { ProjectsIssuesList } from "@/features/projects/ui/ProjectsIssuesList";
import { ProjectsOverviewPanel } from "@/features/projects/ui/ProjectsOverviewPanel";
import { ProjectsPullRequestsList } from "@/features/projects/ui/ProjectsPullRequestsList";
import {
  ProjectsToolbar,
  ProjectsViewModeToggle,
} from "@/features/projects/ui/ProjectsToolbar";
import { hasLocalCheckout } from "@/features/projects/lib/projectLocalRepos";
import {
  formatCreatedDate,
  formatExactTimestamp,
  getActivityLabel,
  getClonePathLabel,
  getProjectUpdatedAt,
  isProjectMine,
  isProjectOwnedByCurrentUser,
  projectHasAgent,
  projectOwnerIsUser,
  projectPeople,
  relativeTime,
  type ProjectsFilter,
  type ProjectsSort,
  type ProjectsViewMode,
  readStoredFilter,
  readStoredSort,
  readStoredViewMode,
  uniqueRepositories,
  writeStoredFilter,
  writeStoredSort,
  writeStoredViewMode,
} from "@/features/projects/lib/projectsViewHelpers";
import {
  projectTerminalLabel,
  useOpenProjectTerminal,
} from "@/features/projects/ui/useOpenProjectTerminal";
import { useWorkspaces } from "@/features/workspaces/useWorkspaces";
import { useIdentityQuery } from "@/shared/api/hooks";
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
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const MANY_PROJECTS_THRESHOLD = 12;

const CLONE_PATH_TAIL_CHARS = 14;

function ClonePathLabel({ project }: { project: Project }) {
  const label = getClonePathLabel(project);
  const fullUrl = project.cloneUrls[0] ?? label;
  const splitAt = Math.max(0, label.length - CLONE_PATH_TAIL_CHARS);
  const head = label.slice(0, splitAt);
  const tail = label.slice(splitAt);

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="relative z-10 flex min-w-0 font-mono">
          <span className="min-w-0 truncate">{head}</span>
          <span className="shrink-0">{tail}</span>
        </span>
      </TooltipTrigger>
      <TooltipContent className="max-w-96 break-all font-mono">
        {fullUrl}
      </TooltipContent>
    </Tooltip>
  );
}

function ClonePathIcon({ project }: { project: Project }) {
  const fullUrl = project.cloneUrls[0] ?? getClonePathLabel(project);

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="relative z-10 flex shrink-0 items-center text-muted-foreground/80">
          <GitFork className="h-3.5 w-3.5" />
        </span>
      </TooltipTrigger>
      <TooltipContent className="max-w-96 break-all font-mono">
        {fullUrl}
      </TooltipContent>
    </Tooltip>
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
  const visible = pubkeys.slice(0, 5);
  const remaining = pubkeys.length - visible.length;

  if (visible.length === 0) {
    return null;
  }

  return (
    <div className="flex items-center justify-end -space-x-1.5">
      {visible.map((pubkey, index) => {
        const profile = profiles?.[normalizePubkey(pubkey)];
        const label = resolveUserLabel({ pubkey, profiles });
        return (
          <Tooltip key={pubkey}>
            <TooltipTrigger asChild>
              {/* First avatar sits on the top layer, cascading down rightward. */}
              <span
                className="relative inline-flex"
                style={{ zIndex: visible.length - index }}
              >
                <UserAvatar
                  accent={
                    normalizePubkey(pubkey) === normalizePubkey(workOwnerPubkey)
                  }
                  avatarUrl={profile?.avatarUrl ?? null}
                  className="ring-2 ring-card"
                  displayName={label}
                  size="xs"
                />
              </span>
            </TooltipTrigger>
            <TooltipContent>{label}</TooltipContent>
          </Tooltip>
        );
      })}
      {remaining > 0 ? (
        <span className="relative z-0 flex h-5 min-w-5 items-center justify-center rounded-full bg-muted px-1 text-3xs font-semibold text-muted-foreground ring-2 ring-card">
          +{remaining}
        </span>
      ) : null}
    </div>
  );
}

function LatestCommitLabel({
  profiles,
  project,
  summary,
}: {
  profiles?: UserProfileLookup;
  project: Project;
  summary: ProjectActivitySummary | undefined;
}) {
  const latestCommit = summary?.latestCommit;
  if (!latestCommit) {
    return (
      <MetadataItem icon={CalendarDays}>
        {formatCreatedDate(project.createdAt)}
      </MetadataItem>
    );
  }

  const authorLabel = latestCommit.author
    ? resolveUserLabel({ profiles, pubkey: latestCommit.author })
    : null;
  const commitLabel = latestCommit.title || latestCommit.commit.slice(0, 7);
  // Middle truncation: the commit title gives way while the trailing
  // author + time stay pinned and always visible.
  const tail = `${authorLabel ? ` · ${authorLabel}` : ""} · ${relativeTime(
    latestCommit.createdAt,
  )}`;

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="flex min-w-0 items-center gap-1.5">
          <GitCommit className="h-3.5 w-3.5 shrink-0 text-muted-foreground/70" />
          <span className="flex min-w-0">
            <span className="min-w-0 truncate">{commitLabel}</span>
            <span className="shrink-0 whitespace-pre">{tail}</span>
          </span>
        </span>
      </TooltipTrigger>
      <TooltipContent className="max-w-96 break-words">
        {commitLabel}
        {authorLabel ? ` · ${authorLabel}` : ""} ·{" "}
        {formatExactTimestamp(latestCommit.createdAt)}
      </TooltipContent>
    </Tooltip>
  );
}

function StatusPill({ status }: { status: string }) {
  if (status === "active") {
    return null;
  }

  return (
    <span className="shrink-0 rounded-full bg-muted/60 px-2 pb-[3px] pt-[5px] text-2xs font-semibold uppercase leading-none tracking-[0.18em] text-muted-foreground">
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
  hasLocal,
  canDelete,
  disabled,
  onDelete,
  onOpenTerminal,
}: {
  project: Project;
  hasLocal: boolean;
  canDelete: boolean;
  disabled: boolean;
  onDelete: (project: Project) => Promise<void> | void;
  onOpenTerminal: (project: Project) => Promise<void> | void;
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
            onSelect={(event) => {
              event.preventDefault();
              event.stopPropagation();
              void onOpenTerminal(project);
            }}
          >
            <TerminalSquare className="h-4 w-4" />
            {projectTerminalLabel(hasLocal)}
          </DropdownMenuItem>
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
  hasLocal,
  canDelete,
  deleteDisabled,
  onDelete,
  onOpen,
  onOpenTerminal,
}: {
  project: Project;
  people: string[];
  profiles?: UserProfileLookup;
  summary: ProjectActivitySummary | undefined;
  hasLocal: boolean;
  canDelete: boolean;
  deleteDisabled: boolean;
  onDelete: (project: Project) => Promise<void> | void;
  onOpen: (project: Project) => void;
  onOpenTerminal: (project: Project) => Promise<void> | void;
}) {
  return (
    <Card
      className="group relative flex min-h-48 flex-col overflow-hidden rounded-2xl border-border/50 bg-muted/20 p-4 shadow-none transition-colors duration-150 hover:bg-muted/30"
      data-testid={`project-card-${project.dtag}`}
    >
      <ProjectCardButton onOpen={onOpen} project={project} />
      <div className="flex min-h-0 flex-1 flex-col gap-3">
        <div className="flex min-w-0 items-start justify-between gap-3">
          <div className="min-w-0 space-y-1.5">
            <div className="flex min-w-0 items-center gap-2">
              <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60">
                <FolderGit2 className="h-4 w-4 text-muted-foreground" />
              </span>
              <div className="min-w-0 space-y-1">
                <span className="block truncate text-sm font-semibold text-foreground">
                  {project.name}
                </span>
                <div className="flex min-w-0 items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                  <LatestCommitLabel
                    profiles={profiles}
                    project={project}
                    summary={summary}
                  />
                </div>
              </div>
            </div>
          </div>
          <div className="relative z-10 flex items-center gap-1">
            <StatusPill status={project.status} />
            <ProjectActionsMenu
              canDelete={canDelete}
              disabled={deleteDisabled}
              hasLocal={hasLocal}
              onDelete={onDelete}
              onOpenTerminal={onOpenTerminal}
              project={project}
            />
          </div>
        </div>

        <p className="line-clamp-2 min-h-10 text-sm text-muted-foreground">
          {project.description || "A shared space for internal git work."}
        </p>

        <div className="mt-auto rounded-xl bg-muted/60 px-2.5 py-2">
          <div className="flex min-w-0 items-center justify-between gap-2">
            <div className="relative z-10 flex shrink-0 items-center gap-1">
              <ProjectPeopleStack
                profiles={profiles}
                pubkeys={people}
                workOwnerPubkey={project.owner}
              />
            </div>
            <div className="flex min-w-0 items-center gap-2">
              <p className="truncate text-xs font-medium text-muted-foreground">
                {getActivityLabel(summary)}
              </p>
              <ClonePathIcon project={project} />
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
  hasLocal,
  canDelete,
  deleteDisabled,
  onDelete,
  onOpen,
  onOpenTerminal,
}: {
  project: Project;
  people: string[];
  profiles?: UserProfileLookup;
  summary: ProjectActivitySummary | undefined;
  hasLocal: boolean;
  canDelete: boolean;
  deleteDisabled: boolean;
  onDelete: (project: Project) => Promise<void> | void;
  onOpen: (project: Project) => void;
  onOpenTerminal: (project: Project) => Promise<void> | void;
}) {
  return (
    <Card
      className="group relative overflow-hidden rounded-2xl border-border/50 bg-muted/20 p-3 shadow-none transition-colors duration-150 hover:bg-muted/30"
      data-testid={`project-row-${project.dtag}`}
    >
      <ProjectCardButton onOpen={onOpen} project={project} />
      <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_16rem] lg:items-center">
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
            <ClonePathLabel project={project} />
          </div>
        </div>

        <div className="relative z-10 flex min-w-0 items-center gap-2 rounded-xl bg-muted/60 px-2.5 py-2 lg:w-full">
          <p className="min-w-0 flex-1 truncate text-xs font-medium text-muted-foreground">
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
            hasLocal={hasLocal}
            onDelete={onDelete}
            onOpenTerminal={onOpenTerminal}
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
  const projectPullRequestsQuery = useProjectsPullRequestsQuery(projects);
  const [filter, setFilter] = React.useState<ProjectsFilter>(() =>
    readStoredFilter(),
  );
  const projectIssuesQuery = useProjectsIssuesQuery(
    filter === "issues" ? projects : [],
  );
  const [storedViewMode, setStoredViewMode] =
    React.useState<ProjectsViewMode | null>(() => readStoredViewMode());
  const [sort, setSort] = React.useState<ProjectsSort>(() => readStoredSort());
  const viewMode =
    storedViewMode ??
    (projects.length > MANY_PROJECTS_THRESHOLD ? "list" : "grid");

  const projectPubkeys = React.useMemo(
    () => [
      ...new Set(
        [
          ...projects.flatMap((project) =>
            projectPeople(
              project,
              activitySummariesQuery.data?.[project.repoAddress],
            ),
          ),
          ...(projectPullRequestsQuery.data?.flatMap(({ pullRequest }) => [
            pullRequest.author,
            ...pullRequest.recipients,
            ...pullRequest.comments.map((comment) => comment.author),
          ]) ?? []),
        ].map(normalizePubkey),
      ),
    ],
    [activitySummariesQuery.data, projectPullRequestsQuery.data, projects],
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

  const localRepoNames = React.useMemo(
    () =>
      new Set(
        (localRepositoriesQuery.data ?? []).map(
          (repository) => repository.name,
        ),
      ),
    [localRepositoriesQuery.data],
  );

  // Count projects with a checkout on this machine — matches what the
  // "Local" filter actually lists, not every directory in the repos folder.
  const localProjectCount = React.useMemo(
    () =>
      projects.filter((project) => hasLocalCheckout(project, localRepoNames))
        .length,
    [localRepoNames, projects],
  );

  const visibleProjects = React.useMemo(() => {
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
    localRepoNames,
    profiles,
    projects,
    sort,
  ]);

  const visiblePullRequests = React.useMemo(() => {
    const pullRequests = projectPullRequestsQuery.data ?? [];
    return [...pullRequests].sort((left, right) => {
      if (sort === "name") {
        return left.pullRequest.title.localeCompare(right.pullRequest.title);
      }
      if (sort === "created") {
        return right.pullRequest.createdAt - left.pullRequest.createdAt;
      }
      return right.pullRequest.updatedAt - left.pullRequest.updatedAt;
    });
  }, [projectPullRequestsQuery.data, sort]);

  const visibleIssues = React.useMemo(() => {
    const issues = projectIssuesQuery.data ?? [];
    return [...issues].sort((left, right) => {
      if (sort === "name") {
        return left.issue.title.localeCompare(right.issue.title);
      }
      if (sort === "created") {
        return right.issue.createdAt - left.issue.createdAt;
      }
      return right.issue.updatedAt - left.issue.updatedAt;
    });
  }, [projectIssuesQuery.data, sort]);

  const handleOpenProject = React.useCallback(
    (project: Project) => {
      void goProject(project.dtag);
    },
    [goProject],
  );

  const handleOpenPullRequest = React.useCallback(
    (project: Project, pullRequest: ProjectPullRequest) => {
      void goProject(project.dtag, { pullRequestId: pullRequest.id });
    },
    [goProject],
  );

  const handleOpenIssue = React.useCallback(
    (project: Project, issue: ProjectIssue) => {
      void goProject(project.dtag, { issueId: issue.id });
    },
    [goProject],
  );

  const openTerminal = useOpenProjectTerminal(activeWorkspace?.reposDir);
  const handleOpenTerminal = React.useCallback(
    (project: Project) =>
      openTerminal(project, {
        hasLocalCheckout: hasLocalCheckout(project, localRepoNames),
      }),
    [localRepoNames, openTerminal],
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
        />
      </div>

      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto px-4 pb-4">
        <div className="pt-[calc(var(--buzz-channel-content-top-padding,5.75rem)_+_1rem)]">
          {filter === "all" ? (
            <ProjectsOverviewPanel
              localRepositoryCount={localProjectCount}
              onSelectSection={handleFilterChange}
              profiles={profiles}
              projects={projects}
              relayName={activeWorkspace?.name || "Relay"}
              summaries={activitySummariesQuery.data}
            />
          ) : null}
          <section className="space-y-3">
            <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
              <h3 className="text-sm font-semibold text-foreground">
                {filter === "prs"
                  ? "Pull requests"
                  : filter === "issues"
                    ? "Issues"
                    : "Repositories"}
              </h3>
              <div className="flex flex-wrap items-center gap-2 sm:justify-end">
                <label className="flex items-center gap-2 text-xs text-muted-foreground">
                  <span className="sr-only">Sort projects</span>
                  <select
                    className="h-8 rounded-md bg-background px-2 text-xs text-foreground outline-hidden focus:ring-1 focus:ring-ring"
                    onChange={(event) =>
                      handleSortChange(event.target.value as ProjectsSort)
                    }
                    value={sort}
                  >
                    <option value="updated">Recent activity</option>
                    <option value="created">Created date</option>
                    <option value="name">Name</option>
                  </select>
                </label>
                <ProjectsViewModeToggle
                  onViewModeChange={handleViewModeChange}
                  viewMode={viewMode}
                />
              </div>
            </div>
            {filter === "prs" ? (
              <ProjectsPullRequestsList
                isLoading={projectPullRequestsQuery.isLoading}
                onOpen={handleOpenPullRequest}
                profiles={profiles}
                pullRequests={visiblePullRequests}
                viewMode={viewMode}
              />
            ) : filter === "issues" ? (
              <ProjectsIssuesList
                isLoading={projectIssuesQuery.isLoading}
                issues={visibleIssues}
                onOpen={handleOpenIssue}
                profiles={profiles}
                viewMode={viewMode}
              />
            ) : visibleProjects.length === 0 ? (
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
                      hasLocal={hasLocalCheckout(project, localRepoNames)}
                      key={project.id}
                      onDelete={handleDeleteProject}
                      onOpen={handleOpenProject}
                      onOpenTerminal={handleOpenTerminal}
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
                      hasLocal={hasLocalCheckout(project, localRepoNames)}
                      key={project.id}
                      onDelete={handleDeleteProject}
                      onOpen={handleOpenProject}
                      onOpenTerminal={handleOpenTerminal}
                      people={projectPeople(project, summary)}
                      profiles={profiles}
                      project={project}
                      summary={summary}
                    />
                  );
                })}
              </div>
            )}
          </section>
        </div>
      </div>
    </div>
  );
}
