import { Search } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import {
  type Project,
  type ProjectIssue,
  type ProjectPullRequest,
  type Repository,
  useDeleteProjectMutation,
  useProjectActivitySummariesQuery,
  useProjectLocalRepositoriesQuery,
  useProjectsQuery,
  useProjectsWorkItemsQuery,
} from "@/features/projects/hooks";
import { useRepositoryActivitySummariesQuery } from "@/features/projects/repositoryActivityHooks";
import { useCreateProjectMutation } from "@/features/projects/useCreateProject";
import { selectProjectRepository } from "@/features/projects/projectModels";
import { useProjectsRepoSnapshotsQuery } from "@/features/projects/useProjectsRepoSnapshots";
import {
  useMemberChannelIds,
  useRepositoryUnavailableReasonFor,
} from "@/features/projects/useRepositoryAccess";
import {
  projectRepoHostForProject,
  projectRepoHostForRepository,
} from "@/features/projects/lib/projectRepoHost";
import { ProjectsActivityFeed } from "@/features/projects/ui/ProjectsActivityFeed";
import { ProjectsChannelsList } from "@/features/projects/ui/ProjectsChannelsList";
import { ProjectsOverviewContextSheet } from "@/features/projects/ui/ProjectsOverviewContextSheet";
import {
  ProjectsActivityIntro,
  ProjectsOverviewContextPanel,
  ProjectsOverviewPanel,
} from "@/features/projects/ui/ProjectsOverviewPanel";
import {
  openAppSearch,
  projectsSectionIcon,
  projectsSectionTitle,
} from "@/features/projects/ui/projectsSectionMeta";
import { EmptyState } from "@/features/projects/ui/ProjectCards";
import {
  ProjectsOverviewProjectItems,
  ProjectsOverviewRepositoryItems,
} from "@/features/projects/ui/ProjectsOverviewItems";
import { CreateProjectDialog } from "@/features/projects/ui/CreateProjectDialog";
import { CreateProjectIssueDialog } from "@/features/projects/ui/CreateProjectIssueDialog";
import { CreatePullRequestDialog } from "@/features/projects/ui/CreatePullRequestDialog";
import { ProjectsCreateMenu } from "@/features/projects/ui/ProjectsCreateMenu";
import { ProjectsIssuesList } from "@/features/projects/ui/ProjectsIssuesList";
import { ProjectsWorkspaceChrome } from "@/features/projects/ui/ProjectDetailChrome";
import { ProjectsPullRequestsList } from "@/features/projects/ui/ProjectsPullRequestsList";
import { ProjectsWorkItemsLoadNotice } from "@/features/projects/ui/ProjectsWorkItemsLoadNotice";
import { ProjectsListHeaderBar } from "@/features/projects/ui/ProjectsListHeaderBar";
import { ProjectSectionHeader } from "@/features/projects/ui/ProjectSectionHeader";
import { PROJECT_COLUMN_HEADER_BACKDROP_CLASS } from "@/features/projects/ui/projectPanelStyles";
import { ProjectsToolbar } from "@/features/projects/ui/ProjectsToolbar";
import {
  hasLocalCheckout,
  hasLocalRepositoryCheckout,
} from "@/features/projects/lib/projectLocalRepos";
import {
  getProjectUpdatedAt,
  isProjectAccessibleToViewer,
  isProjectMine,
  isRepositoryAccessibleToViewer,
  projectHasAgent,
  projectOwnerIsUser,
  projectPeople,
  type ProjectsFilter,
  type ProjectsRepositoryScope,
  type ProjectsSort,
  type ProjectsViewMode,
  type ProjectsWorkItemScope,
  readStoredFilter,
  readStoredIssueScope,
  readStoredPullRequestScope,
  readStoredRepositoryScope,
  readStoredSort,
  readStoredViewMode,
  writeStoredFilter,
  writeStoredIssueScope,
  writeStoredPullRequestScope,
  writeStoredRepositoryScope,
  writeStoredSort,
  writeStoredViewMode,
} from "@/features/projects/lib/projectsViewHelpers";
import { useOpenProjectTerminal } from "@/features/projects/ui/useOpenProjectTerminal";
import { PROJECT_CONTEXT_PANEL_DEFAULT_WIDTH_PX } from "@/features/projects/ui/useProjectPanelWidths";
import { useMediaBreakpoint } from "@/shared/hooks/use-mobile";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import { topChromeInset } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { useRelayOrigin } from "@/shared/lib/useRelayOrigin";
import { Button } from "@/shared/ui/button";
import { DrawerPanelIcon } from "@/shared/ui/DrawerPanelIcon";

const MANY_PROJECTS_THRESHOLD = 12;
const PROJECTS_CONTEXT_POD_MIN_VIEWPORT_PX = 1024;

export function ProjectsView() {
  const { goProject } = useAppNavigation();
  const { activeCommunity } = useCommunities();
  const relayOrigin = useRelayOrigin();
  const scrollIdleTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const scrollIndicatorRef = React.useRef<HTMLDivElement | null>(null);
  // The native scrollbar thumb is permanently transparent (WebKit won't
  // re-resolve ::-webkit-scrollbar styles dynamically), so we paint our own
  // indicator over the gutter and show it only while the area is scrolling.
  const handleContentScroll = React.useCallback(
    (event: React.UIEvent<HTMLDivElement>) => {
      const element = event.currentTarget;
      const indicator = scrollIndicatorRef.current;
      if (!indicator) return;

      const { clientHeight, scrollHeight, scrollTop } = element;
      if (scrollHeight <= clientHeight) {
        indicator.style.opacity = "0";
        return;
      }

      const thumbHeight = Math.max(
        24,
        (clientHeight / scrollHeight) * clientHeight,
      );
      const maxOffset = clientHeight - thumbHeight;
      const offset = (scrollTop / (scrollHeight - clientHeight)) * maxOffset;
      indicator.style.height = `${thumbHeight}px`;
      indicator.style.transform = `translateY(${offset}px)`;
      indicator.style.opacity = "1";

      if (scrollIdleTimerRef.current !== null) {
        globalThis.clearTimeout(scrollIdleTimerRef.current);
      }
      scrollIdleTimerRef.current = globalThis.setTimeout(() => {
        indicator.style.opacity = "0";
        scrollIdleTimerRef.current = null;
      }, 700);
    },
    [],
  );
  const projectsQuery = useProjectsQuery();
  const identityQuery = useIdentityQuery();
  const projects = projectsQuery.data ?? [];
  const localRepositoriesQuery = useProjectLocalRepositoriesQuery(
    activeCommunity?.reposDir,
  );
  const [filter, setFilter] = React.useState<ProjectsFilter>(() => {
    const storedFilter = readStoredFilter();
    return storedFilter === "mine" || storedFilter === "local"
      ? "repositories"
      : storedFilter;
  });
  const [overviewPanelOpen, setOverviewPanelOpen] = React.useState(true);
  // Narrow layouts present the same context as a dismissible sheet instead of
  // the docked rail; the sheet starts closed so resizing never pops a modal.
  const [narrowContextOpen, setNarrowContextOpen] = React.useState(false);
  const contextToggleRef = React.useRef<HTMLButtonElement | null>(null);
  const isNarrowProjectsLayout = useMediaBreakpoint(
    PROJECTS_CONTEXT_POD_MIN_VIEWPORT_PX,
  );
  const activitySummariesQuery = useProjectActivitySummariesQuery(projects);
  const repositoryActivitySummariesQuery = useRepositoryActivitySummariesQuery(
    filter === "repositories" ? projects : [],
  );
  const [repositoryScope, setRepositoryScope] =
    React.useState<ProjectsRepositoryScope>(() => {
      const storedScope = readStoredRepositoryScope();
      return filter === "projects" &&
        (storedScope === "buzz" || storedScope === "linked")
        ? "all"
        : storedScope;
    });
  const [pullRequestScope, setPullRequestScope] =
    React.useState<ProjectsWorkItemScope>(() => readStoredPullRequestScope());
  const [issueScope, setIssueScope] = React.useState<ProjectsWorkItemScope>(
    () => readStoredIssueScope(),
  );
  const projectsWorkItemsQuery = useProjectsWorkItemsQuery(projects);
  // One blobless clone per primary Buzz repository, only while the overview
  // header is visible.
  const snapshotProjects = React.useMemo(
    () =>
      filter === "all"
        ? projects.filter(
            (project) =>
              projectRepoHostForProject(project, relayOrigin).kind === "buzz",
          )
        : [],
    [filter, projects, relayOrigin],
  );
  const repoSnapshotsQuery = useProjectsRepoSnapshotsQuery(
    snapshotProjects,
    activeCommunity?.reposDir,
  );
  const memberChannelIds = useMemberChannelIds();
  const repositoryUnavailableReasonFor = useRepositoryUnavailableReasonFor(
    repoSnapshotsQuery.data?.unavailable,
    memberChannelIds,
  );
  const [createProjectOpen, setCreateProjectOpen] = React.useState(false);
  const [createIssueOpen, setCreateIssueOpen] = React.useState(false);
  const [createPullRequestOpen, setCreatePullRequestOpen] =
    React.useState(false);
  const createProjectMutation = useCreateProjectMutation();
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
            projectPeople(project, activitySummariesQuery.data?.[project.id]),
          ),
          ...(projectsWorkItemsQuery.data?.pullRequests.items.flatMap(
            ({ pullRequest }) => [
              pullRequest.author,
              ...pullRequest.recipients,
              ...pullRequest.reviewers,
              ...pullRequest.approvals.map((approval) => approval.author),
              ...pullRequest.updates.map((update) => update.author),
              ...pullRequest.comments.map((comment) => comment.author),
            ],
          ) ?? []),
          ...(projectsWorkItemsQuery.data?.issues.items.flatMap(({ issue }) => [
            issue.author,
            ...issue.recipients,
            ...issue.assignees,
            ...issue.comments.map((comment) => comment.author),
          ]) ?? []),
        ].map(normalizePubkey),
      ),
    ],
    [activitySummariesQuery.data, projects, projectsWorkItemsQuery.data],
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

  const handleFilterChange = React.useCallback(
    (nextFilter: ProjectsFilter) => {
      if (
        nextFilter === "projects" &&
        (repositoryScope === "buzz" || repositoryScope === "linked")
      ) {
        setRepositoryScope("all");
        writeStoredRepositoryScope("all");
      }
      setFilter(nextFilter);
      writeStoredFilter(nextFilter);
    },
    [repositoryScope],
  );

  const handleRepositoryScopeChange = React.useCallback(
    (scope: ProjectsRepositoryScope) => {
      setRepositoryScope(scope);
      writeStoredRepositoryScope(scope);
    },
    [],
  );

  const handlePullRequestScopeChange = React.useCallback(
    (scope: ProjectsWorkItemScope) => {
      setPullRequestScope(scope);
      writeStoredPullRequestScope(scope);
    },
    [],
  );

  const handleIssueScopeChange = React.useCallback(
    (scope: ProjectsWorkItemScope) => {
      setIssueScope(scope);
      writeStoredIssueScope(scope);
    },
    [],
  );

  const handleSortChange = React.useCallback((nextSort: ProjectsSort) => {
    setSort(nextSort);
    writeStoredSort(nextSort);
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

  const repositoryAccessInput = React.useMemo(
    () => ({
      currentPubkey,
      localRepoNames,
      memberChannelIds,
      relayOrigin,
    }),
    [currentPubkey, localRepoNames, memberChannelIds, relayOrigin],
  );

  const visibleProjects = React.useMemo(() => {
    if (filter !== "projects" && filter !== "agents" && filter !== "users") {
      return [];
    }

    const sortedProjects = projects
      .filter((project) => {
        const summary = activitySummariesQuery.data?.[project.id];
        const people = projectPeople(project, summary);
        if (repositoryScope === "accessible")
          return isProjectAccessibleToViewer(project, repositoryAccessInput);
        if (repositoryScope === "mine")
          return isProjectMine(project, currentPubkey);
        if (repositoryScope === "local")
          return hasLocalCheckout(project, localRepoNames);
        if (repositoryScope === "buzz")
          return (
            projectRepoHostForProject(project, relayOrigin).kind === "buzz"
          );
        if (repositoryScope === "linked")
          return (
            projectRepoHostForProject(project, relayOrigin).kind === "external"
          );
        if (filter === "agents") {
          return projectHasAgent(project, people, profiles);
        }
        if (filter === "users") return projectOwnerIsUser(project, profiles);
        return true;
      })
      .sort((left, right) => {
        const leftSummary = activitySummariesQuery.data?.[left.id];
        const rightSummary = activitySummariesQuery.data?.[right.id];
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

    return sortedProjects;
  }, [
    activitySummariesQuery.data,
    currentPubkey,
    filter,
    localRepoNames,
    profiles,
    projects,
    relayOrigin,
    repositoryAccessInput,
    repositoryScope,
    sort,
  ]);

  const visibleRepositories = React.useMemo(() => {
    if (filter !== "repositories") return [];
    const repositories = [
      ...new Map(
        projects
          .flatMap((project) =>
            project.repositories.map((repository) => ({
              project,
              repository,
            })),
          )
          .map((item) => [item.repository.repoAddress, item]),
      ).values(),
    ];
    return repositories
      .filter(({ repository }) => {
        if (repositoryScope === "accessible") {
          return isRepositoryAccessibleToViewer(
            repository,
            repositoryAccessInput,
          );
        }
        if (repositoryScope === "mine") {
          if (!currentPubkey) return false;
          const normalizedCurrentPubkey = normalizePubkey(currentPubkey);
          return (
            normalizePubkey(repository.owner) === normalizedCurrentPubkey ||
            repository.contributors.some(
              (pubkey) => normalizePubkey(pubkey) === normalizedCurrentPubkey,
            )
          );
        }
        if (repositoryScope === "local") {
          return hasLocalRepositoryCheckout(repository, localRepoNames);
        }
        if (repositoryScope === "buzz") {
          return (
            projectRepoHostForRepository(repository, relayOrigin).kind ===
            "buzz"
          );
        }
        if (repositoryScope === "linked") {
          return (
            projectRepoHostForRepository(repository, relayOrigin).kind ===
            "external"
          );
        }
        return true;
      })
      .sort((left, right) => {
        if (sort === "name") {
          return left.repository.name.localeCompare(right.repository.name);
        }
        if (sort === "created") {
          return right.repository.createdAt - left.repository.createdAt;
        }
        const leftUpdatedAt =
          repositoryActivitySummariesQuery.data?.[left.repository.repoAddress]
            ?.updatedAt ?? left.repository.createdAt;
        const rightUpdatedAt =
          repositoryActivitySummariesQuery.data?.[right.repository.repoAddress]
            ?.updatedAt ?? right.repository.createdAt;
        return rightUpdatedAt - leftUpdatedAt;
      });
  }, [
    currentPubkey,
    filter,
    localRepoNames,
    projects,
    relayOrigin,
    repositoryAccessInput,
    repositoryActivitySummariesQuery.data,
    repositoryScope,
    sort,
  ]);

  const visiblePullRequests = React.useMemo(() => {
    const pullRequests = projectsWorkItemsQuery.data?.pullRequests.items ?? [];
    const scopedPullRequests =
      pullRequestScope === "mine" && currentPubkey
        ? pullRequests.filter(
            ({ pullRequest }) =>
              normalizePubkey(pullRequest.author) ===
              normalizePubkey(currentPubkey),
          )
        : pullRequests;
    return [...scopedPullRequests].sort((left, right) => {
      if (sort === "name") {
        return left.pullRequest.title.localeCompare(right.pullRequest.title);
      }
      if (sort === "created") {
        return right.pullRequest.createdAt - left.pullRequest.createdAt;
      }
      return right.pullRequest.updatedAt - left.pullRequest.updatedAt;
    });
  }, [currentPubkey, projectsWorkItemsQuery.data, pullRequestScope, sort]);

  const visibleIssues = React.useMemo(() => {
    const issues = projectsWorkItemsQuery.data?.issues.items ?? [];
    const viewer = currentPubkey ? normalizePubkey(currentPubkey) : null;
    const scopedIssues =
      issueScope === "mine" && viewer
        ? issues.filter(({ issue }) => normalizePubkey(issue.author) === viewer)
        : issueScope === "assigned" && viewer
          ? issues.filter(({ issue }) =>
              issue.assignees.some(
                (assignee) => normalizePubkey(assignee) === viewer,
              ),
            )
          : issues;
    return [...scopedIssues].sort((left, right) => {
      if (sort === "name") {
        return left.issue.title.localeCompare(right.issue.title);
      }
      if (sort === "created") {
        return right.issue.createdAt - left.issue.createdAt;
      }
      return right.issue.updatedAt - left.issue.updatedAt;
    });
  }, [currentPubkey, issueScope, projectsWorkItemsQuery.data, sort]);

  // Route by the canonical `owner:dtag` project ID — a bare dtag is
  // ambiguous across owners (forks can share the same dtag).
  const handleOpenProject = React.useCallback(
    (project: Project) => {
      void goProject(project.id);
    },
    [goProject],
  );

  const handleOpenRepository = React.useCallback(
    (project: Project, repository: Repository) => {
      void goProject(project.id, { repositoryId: repository.id });
    },
    [goProject],
  );

  const handleOpenCommit = React.useCallback(
    (project: Project, commitHash: string) => {
      void goProject(project.id, { commitHash });
    },
    [goProject],
  );

  const handleOpenPullRequest = React.useCallback(
    (
      project: Project,
      repository: Repository,
      pullRequest: ProjectPullRequest,
    ) => {
      void goProject(project.id, {
        pullRequestId: pullRequest.id,
        repositoryId: repository.id,
      });
    },
    [goProject],
  );

  const handleOpenIssue = React.useCallback(
    (project: Project, repository: Repository, issue: ProjectIssue) => {
      void goProject(project.id, {
        issueId: issue.id,
        repositoryId: repository.id,
      });
    },
    [goProject],
  );

  const openTerminal = useOpenProjectTerminal(activeCommunity?.reposDir);
  const handleOpenTerminal = React.useCallback(
    (project: Project) => {
      const repository = selectProjectRepository(project, null);
      if (!repository) return Promise.resolve();
      return openTerminal(repository, {
        // Check the selected repository only — not all members — so the
        // terminal affordance reflects the repository the button will open.
        hasLocalCheckout: hasLocalRepositoryCheckout(
          repository,
          localRepoNames,
        ),
      });
    },
    [localRepoNames, openTerminal],
  );
  const handleOpenRepositoryTerminal = React.useCallback(
    (repository: Repository) =>
      openTerminal(repository, {
        hasLocalCheckout: hasLocalRepositoryCheckout(
          repository,
          localRepoNames,
        ),
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
    return <ViewLoadingFallback kind="projects" />;
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

  const projectItems = (
    <ProjectsOverviewProjectItems
      currentPubkey={currentPubkey}
      deleteDisabled={deleteProjectMutation.isPending}
      filter={filter}
      localRepoNames={localRepoNames}
      onDelete={handleDeleteProject}
      onOpen={handleOpenProject}
      onOpenTerminal={handleOpenTerminal}
      profiles={profiles}
      repositoryUnavailableReasonFor={repositoryUnavailableReasonFor}
      summaries={activitySummariesQuery.data}
      viewMode={viewMode}
      visibleProjects={visibleProjects}
    />
  );

  const repositoryItems = (
    <ProjectsOverviewRepositoryItems
      localRepoNames={localRepoNames}
      onOpen={handleOpenRepository}
      onOpenTerminal={handleOpenRepositoryTerminal}
      profiles={profiles}
      summaries={repositoryActivitySummariesQuery.data}
      viewMode={viewMode}
      visibleRepositories={visibleRepositories}
    />
  );

  const listHeaderBar = (
    <ProjectsListHeaderBar
      filter={filter}
      variant="bar"
      issueScope={issueScope}
      onIssueScopeChange={handleIssueScopeChange}
      onPullRequestScopeChange={handlePullRequestScopeChange}
      onRepositoryScopeChange={handleRepositoryScopeChange}
      onSortChange={handleSortChange}
      onViewModeChange={handleViewModeChange}
      pullRequestScope={pullRequestScope}
      repositoryScope={repositoryScope}
      sort={sort}
      viewMode={viewMode}
    />
  );

  const workItemFailedSections = [
    ...new Set([
      ...(projectsWorkItemsQuery.data?.issues.failedSections ?? []),
      ...(projectsWorkItemsQuery.data?.pullRequests.failedSections ?? []),
    ]),
  ];
  const activityFeed = (
    <>
      <ProjectsWorkItemsLoadNotice
        error={projectsWorkItemsQuery.error}
        failedSections={workItemFailedSections}
        isRetrying={
          projectsWorkItemsQuery.isFetching && !projectsWorkItemsQuery.isLoading
        }
        onRetry={() => void projectsWorkItemsQuery.refetch()}
        subject="project activity"
      />
      <ProjectsActivityFeed
        isLoading={
          repoSnapshotsQuery.isLoading || projectsWorkItemsQuery.isLoading
        }
        issues={projectsWorkItemsQuery.data?.issues.items ?? []}
        onOpenCommit={handleOpenCommit}
        onOpenIssue={handleOpenIssue}
        onOpenProject={handleOpenProject}
        onOpenPullRequest={handleOpenPullRequest}
        profiles={profiles}
        projects={projects}
        pullRequests={projectsWorkItemsQuery.data?.pullRequests.items ?? []}
        snapshots={repoSnapshotsQuery.data?.snapshots}
      />
    </>
  );

  const createMenu = (
    <ProjectsCreateMenu
      onCreateIssue={() => setCreateIssueOpen(true)}
      onCreateProject={() => setCreateProjectOpen(true)}
      onCreatePullRequest={() => setCreatePullRequestOpen(true)}
    />
  );

  const contextPanelProps = {
    filter,
    issues:
      projectsWorkItemsQuery.data?.issues.items.map(({ issue }) => issue) ?? [],
    onCreateIssue: () => setCreateIssueOpen(true),
    onCreateProject: () => setCreateProjectOpen(true),
    onCreatePullRequest: () => setCreatePullRequestOpen(true),
    profiles,
    projects,
    pullRequests:
      projectsWorkItemsQuery.data?.pullRequests.items.map(
        ({ pullRequest }) => pullRequest,
      ) ?? [],
    summaries: activitySummariesQuery.data,
  };

  const overviewDetached = overviewPanelOpen && !isNarrowProjectsLayout;
  const contextOpen = isNarrowProjectsLayout
    ? narrowContextOpen
    : overviewPanelOpen;
  const chromeActions = (
    <>
      <Button
        aria-label={
          contextOpen ? "Hide project context" : "Show project context"
        }
        aria-pressed={contextOpen}
        className="h-7 w-7 text-sidebar-foreground/65 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
        data-testid="projects-overview-context-toggle"
        onClick={() =>
          isNarrowProjectsLayout
            ? setNarrowContextOpen((open) => !open)
            : setOverviewPanelOpen((open) => !open)
        }
        ref={contextToggleRef}
        size="icon"
        title={contextOpen ? "Hide project context" : "Show project context"}
        type="button"
        variant="ghost"
      >
        <DrawerPanelIcon
          className="-scale-x-100"
          side={contextOpen ? "left" : "right"}
        />
      </Button>
      {overviewDetached ? null : createMenu}
    </>
  );

  return (
    <div
      className={cn(
        "relative flex min-h-0 min-w-0 flex-1 flex-row overflow-hidden",
        overviewDetached && "gap-2 bg-sidebar pb-2 pr-2 pt-px",
      )}
      data-project-context-detached={overviewDetached ? "true" : undefined}
      data-testid="projects-overview-layout"
    >
      <ProjectsWorkspaceChrome
        actions={chromeActions}
        onGoActivity={() => handleFilterChange("all")}
        section={projectsSectionTitle(filter)}
      />
      <div
        className={cn(
          "relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden",
          overviewDetached
            ? "ml-px rounded-2xl bg-background"
            : cn("rounded-tl-xl", topChromeInset.divider),
        )}
        data-testid={
          overviewDetached ? "projects-overview-content-pod" : undefined
        }
      >
        <CreateProjectDialog
          isCreating={createProjectMutation.isPending}
          onCreate={async (input) => {
            const result = await createProjectMutation.mutateAsync(input);
            if (result.compatibilityWarning) {
              toast.warning("Created as a standalone project", {
                description: result.compatibilityWarning,
              });
            } else {
              toast.success(`Project "${result.project.name}" created.`);
            }
            handleRepositoryScopeChange("all");
            handleFilterChange("projects");
          }}
          onOpenChange={setCreateProjectOpen}
          open={createProjectOpen}
        />
        {createPullRequestOpen ? (
          <CreatePullRequestDialog
            onCreated={async (
              createdProject,
              createdRepository,
              pullRequestId,
            ) => {
              await goProject(createdProject.id, {
                pullRequestId,
                repositoryId: createdRepository.id,
              });
            }}
            onOpenChange={setCreatePullRequestOpen}
            open
            projects={projects}
            reposDir={activeCommunity?.reposDir}
          />
        ) : null}
        <CreateProjectIssueDialog
          onCreated={async (createdProject, createdRepository, issueId) => {
            await goProject(createdProject.id, {
              issueId,
              repositoryId: createdRepository.id,
            });
          }}
          onOpenChange={setCreateIssueOpen}
          open={createIssueOpen}
          projects={projects}
        />
        <div className="relative min-h-0 min-w-0 flex-1">
          <div
            aria-hidden="true"
            className="pointer-events-none absolute right-[3px] top-0 z-50 w-1 rounded-full bg-border/80 opacity-0 transition-opacity duration-200"
            ref={scrollIndicatorRef}
          />
          <div
            className="buzz-content-scrollbar h-full min-h-0 min-w-0 overflow-x-hidden overflow-y-scroll"
            onScroll={handleContentScroll}
          >
            <div className="px-4 pb-4">
              <div className="w-full space-y-3">
                <div
                  className={cn(
                    "sticky top-0 z-30 -mx-4 flex h-13 min-w-0 items-center gap-1.5 px-4",
                    PROJECT_COLUMN_HEADER_BACKDROP_CLASS,
                    overviewDetached && "rounded-t-2xl",
                  )}
                  data-testid="projects-page-tabs"
                >
                  {filter === "all" ? (
                    <Button
                      aria-label="Search everything"
                      className="h-7 w-7 shrink-0 text-muted-foreground hover:text-foreground"
                      data-testid="projects-activity-search"
                      onClick={openAppSearch}
                      size="icon"
                      title="Search everything"
                      type="button"
                      variant="ghost"
                    >
                      <Search className="h-4 w-4" />
                    </Button>
                  ) : null}
                  <ProjectsToolbar
                    filter={filter}
                    onFilterChange={handleFilterChange}
                  />
                </div>
                <div
                  className={
                    filter === "all" ? "mx-auto w-full max-w-xl" : "w-full"
                  }
                >
                  {filter === "all" ? (
                    <ProjectsOverviewPanel>
                      <ProjectsActivityIntro />
                      <section className="space-y-3">{activityFeed}</section>
                    </ProjectsOverviewPanel>
                  ) : (
                    <>
                      <ProjectSectionHeader
                        className="-mx-4"
                        icon={projectsSectionIcon(filter)}
                        testId="projects-page-header"
                        title={projectsSectionTitle(filter)}
                      />
                      <section>
                        <div className="space-y-3">
                          {filter === "channels" ? null : listHeaderBar}
                          {filter === "prs" ? (
                            <ProjectsPullRequestsList
                              embedded={viewMode === "list"}
                              error={projectsWorkItemsQuery.error}
                              failedSections={
                                projectsWorkItemsQuery.data?.pullRequests
                                  .failedSections ?? []
                              }
                              isLoading={projectsWorkItemsQuery.isLoading}
                              isRetrying={
                                projectsWorkItemsQuery.isFetching &&
                                !projectsWorkItemsQuery.isLoading
                              }
                              onOpen={handleOpenPullRequest}
                              onRetry={() =>
                                void projectsWorkItemsQuery.refetch()
                              }
                              profiles={profiles}
                              pullRequests={visiblePullRequests}
                              viewMode={viewMode}
                            />
                          ) : filter === "issues" ? (
                            <ProjectsIssuesList
                              embedded={viewMode === "list"}
                              error={projectsWorkItemsQuery.error}
                              failedSections={
                                projectsWorkItemsQuery.data?.issues
                                  .failedSections ?? []
                              }
                              isLoading={projectsWorkItemsQuery.isLoading}
                              isRetrying={
                                projectsWorkItemsQuery.isFetching &&
                                !projectsWorkItemsQuery.isLoading
                              }
                              issues={visibleIssues}
                              onOpen={handleOpenIssue}
                              onRetry={() =>
                                void projectsWorkItemsQuery.refetch()
                              }
                              profiles={profiles}
                              viewMode={viewMode}
                            />
                          ) : filter === "channels" ? (
                            <ProjectsChannelsList projects={projects} />
                          ) : filter === "projects" ? (
                            projectItems
                          ) : (
                            repositoryItems
                          )}
                        </div>
                      </section>
                    </>
                  )}
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
      {overviewDetached ? (
        <aside
          aria-label="Project context"
          className="relative z-30 flex h-full shrink-0 flex-col overflow-hidden bg-transparent"
          style={{ width: PROJECT_CONTEXT_PANEL_DEFAULT_WIDTH_PX }}
        >
          <div className="min-h-0 flex-1 overflow-y-auto">
            <ProjectsOverviewContextPanel
              {...contextPanelProps}
              onSelectSection={(section) => {
                handleFilterChange(section);
              }}
            />
          </div>
        </aside>
      ) : null}
      {isNarrowProjectsLayout ? (
        <ProjectsOverviewContextSheet
          onCloseAutoFocus={(event) => {
            // Return focus to the chrome toggle so the keyboard journey can
            // continue where it started.
            event.preventDefault();
            contextToggleRef.current?.focus();
          }}
          onOpenChange={setNarrowContextOpen}
          open={narrowContextOpen}
        >
          <ProjectsOverviewContextPanel
            {...contextPanelProps}
            onSelectSection={(section) => {
              handleFilterChange(section);
              setNarrowContextOpen(false);
            }}
          />
        </ProjectsOverviewContextSheet>
      ) : null}
    </div>
  );
}
