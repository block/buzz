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
import {
  EmptyFilteredState,
  EmptyState,
  ProjectGridCard,
  ProjectListRow,
} from "@/features/projects/ui/ProjectCards";
import { CreateProjectDialog } from "@/features/projects/ui/CreateProjectDialog";
import { CreateProjectIssueDialog } from "@/features/projects/ui/CreateProjectIssueDialog";
import { CreatePullRequestDialog } from "@/features/projects/ui/CreatePullRequestDialog";
import { ProjectsCreateMenu } from "@/features/projects/ui/ProjectsCreateMenu";
import { ProjectsIssuesList } from "@/features/projects/ui/ProjectsIssuesList";
import { ProjectsOverviewPanel } from "@/features/projects/ui/ProjectsOverviewPanel";
import { ProjectsOverviewRail } from "@/features/projects/ui/ProjectsOverviewRail";
import { ProjectsPullRequestsList } from "@/features/projects/ui/ProjectsPullRequestsList";
import { ProjectsWorkItemsLoadNotice } from "@/features/projects/ui/ProjectsWorkItemsLoadNotice";
import { ProjectsListHeaderBar } from "@/features/projects/ui/ProjectsListHeaderBar";
import { ProjectsToolbar } from "@/features/projects/ui/ProjectsToolbar";
import {
  hasLocalCheckout,
  hasLocalRepositoryCheckout,
} from "@/features/projects/lib/projectLocalRepos";
import {
  RepositoryGridCard,
  RepositoryListRow,
} from "@/features/projects/ui/RepositoryCards";
import {
  getProjectUpdatedAt,
  isProjectAccessibleToViewer,
  isProjectMine,
  isProjectOwnedByCurrentUser,
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
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import { topChromeInset } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { useRelayOrigin } from "@/shared/lib/useRelayOrigin";
import { Button } from "@/shared/ui/button";
import { PageHeader } from "@/shared/ui/PageHeader";

const MANY_PROJECTS_THRESHOLD = 12;

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
  const activitySummariesQuery = useProjectActivitySummariesQuery(
    filter === "prs" || filter === "issues" || filter === "repositories"
      ? []
      : projects,
  );
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
  const projectsWorkItemsQuery = useProjectsWorkItemsQuery(
    filter === "all" || filter === "prs" || filter === "issues" ? projects : [],
  );
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
    const scopedIssues =
      issueScope === "mine" && currentPubkey
        ? issues.filter(
            ({ issue }) =>
              normalizePubkey(issue.author) === normalizePubkey(currentPubkey),
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

  const projectItems =
    visibleProjects.length === 0 ? (
      <EmptyFilteredState />
    ) : viewMode === "grid" ? (
      <div
        className={cn(
          "grid gap-3 md:grid-cols-2",
          filter !== "all" && "xl:grid-cols-3",
        )}
      >
        {visibleProjects.map((project) => {
          const summary = activitySummariesQuery.data?.[project.id];
          return (
            <ProjectGridCard
              canDelete={isProjectOwnedByCurrentUser(project, currentPubkey)}
              deleteDisabled={deleteProjectMutation.isPending}
              hasLocal={hasLocalCheckout(project, localRepoNames)}
              key={project.id}
              onDelete={handleDeleteProject}
              onOpen={handleOpenProject}
              onOpenTerminal={handleOpenTerminal}
              people={projectPeople(project, summary)}
              profiles={profiles}
              project={project}
              repositoryUnavailableReason={repositoryUnavailableReasonFor(
                project,
              )}
              summary={summary}
            />
          );
        })}
      </div>
    ) : (
      <div
        className="divide-y divide-border/60"
        data-testid="projects-list-container"
      >
        {visibleProjects.map((project) => {
          const summary = activitySummariesQuery.data?.[project.id];
          return (
            <ProjectListRow
              canDelete={isProjectOwnedByCurrentUser(project, currentPubkey)}
              deleteDisabled={deleteProjectMutation.isPending}
              hasLocal={hasLocalCheckout(project, localRepoNames)}
              key={project.id}
              onDelete={handleDeleteProject}
              onOpen={handleOpenProject}
              onOpenTerminal={handleOpenTerminal}
              people={projectPeople(project, summary)}
              profiles={profiles}
              project={project}
              repositoryUnavailableReason={repositoryUnavailableReasonFor(
                project,
              )}
              summary={summary}
            />
          );
        })}
      </div>
    );

  const repositoryItems =
    visibleRepositories.length === 0 ? (
      <EmptyFilteredState />
    ) : viewMode === "grid" ? (
      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
        {visibleRepositories.map(({ project, repository }) => (
          <RepositoryGridCard
            hasLocal={hasLocalRepositoryCheckout(repository, localRepoNames)}
            key={repository.repoAddress}
            onOpen={handleOpenRepository}
            onOpenTerminal={handleOpenRepositoryTerminal}
            profiles={profiles}
            project={project}
            repository={repository}
            summary={
              repositoryActivitySummariesQuery.data?.[repository.repoAddress]
            }
          />
        ))}
      </div>
    ) : (
      <div className="divide-y divide-border/60">
        {visibleRepositories.map(({ project, repository }) => (
          <RepositoryListRow
            hasLocal={hasLocalRepositoryCheckout(repository, localRepoNames)}
            key={repository.repoAddress}
            onOpen={handleOpenRepository}
            onOpenTerminal={handleOpenRepositoryTerminal}
            profiles={profiles}
            project={project}
            repository={repository}
            summary={
              repositoryActivitySummariesQuery.data?.[repository.repoAddress]
            }
          />
        ))}
      </div>
    );

  const listHeaderBar = (
    <ProjectsListHeaderBar
      filter={filter}
      variant={viewMode === "list" ? "row" : "bar"}
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

  const projectsHeader = (
    <PageHeader
      className="pointer-events-auto mb-8"
      description="Set up and manage your projects."
      title="Projects"
    />
  );

  const projectsNavigation = (
    <div className="flex h-[3.25rem] min-w-0 items-center">
      <div className="h-full min-w-0 flex-1 overflow-hidden">
        <ProjectsToolbar filter={filter} onFilterChange={handleFilterChange} />
      </div>
    </div>
  );

  return (
    <div
      className={cn(
        "relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-tl-xl",
        topChromeInset.divider,
      )}
    >
      {/* Scroll indicator painted over the scrollbar gutter; only visible
          while scrolling (native thumb is transparent). */}
      <div
        aria-hidden="true"
        className="pointer-events-none absolute right-[3px] top-0 z-50 w-1 rounded-full bg-border/80 opacity-0 transition-opacity duration-200"
        ref={scrollIndicatorRef}
      />
      {/* Create button pinned to the pane's top-right corner: it never
          scrolls with the page, it just stays put. */}
      <div className="absolute right-4 top-4 z-40">{createMenu}</div>
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
          // Land on the complete project list after creation.
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
      <div
        className="buzz-content-scrollbar min-h-0 min-w-0 flex-1 overflow-x-hidden overflow-y-scroll"
        onScroll={handleContentScroll}
      >
        <div className="px-4 pb-7 pt-7 sm:px-6 sm:pb-8 sm:pt-8">
          <div className="mx-auto w-full max-w-6xl">{projectsHeader}</div>
          <div className="sticky top-0 z-30 -mx-4 bg-background/80 backdrop-blur-xl supports-backdrop-filter:bg-background/65 dark:bg-background/75 dark:supports-backdrop-filter:bg-background/60 sm:-mx-6">
            <div className="px-4 sm:px-6">
              <div className="mx-auto w-full max-w-6xl">
                {projectsNavigation}
              </div>
            </div>
          </div>
          <div className="mx-auto w-full max-w-6xl">
            <div className="w-full min-w-0 pb-4 pt-4">
              {filter === "all" ? (
                <ProjectsOverviewPanel
                  metadata={
                    <ProjectsOverviewRail
                      profiles={profiles}
                      projects={projects}
                      summaries={activitySummariesQuery.data}
                    />
                  }
                  onSelectSection={(section) => {
                    handleFilterChange(section);
                  }}
                  projects={projects}
                  summaries={activitySummariesQuery.data}
                >
                  <section className="space-y-3">{activityFeed}</section>
                </ProjectsOverviewPanel>
              ) : (
                <section>
                  {/* In list view the header is the table's first row inside
                      the bordered container; in card view it is a standalone
                      bar with the cards flowing below. */}
                  <div
                    className={
                      viewMode === "list"
                        ? "overflow-hidden rounded-xl border border-border/60"
                        : "space-y-3"
                    }
                  >
                    {listHeaderBar}
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
                        onRetry={() => void projectsWorkItemsQuery.refetch()}
                        profiles={profiles}
                        pullRequests={visiblePullRequests}
                        viewMode={viewMode}
                      />
                    ) : filter === "issues" ? (
                      <ProjectsIssuesList
                        embedded={viewMode === "list"}
                        error={projectsWorkItemsQuery.error}
                        failedSections={
                          projectsWorkItemsQuery.data?.issues.failedSections ??
                          []
                        }
                        isLoading={projectsWorkItemsQuery.isLoading}
                        isRetrying={
                          projectsWorkItemsQuery.isFetching &&
                          !projectsWorkItemsQuery.isLoading
                        }
                        issues={visibleIssues}
                        onOpen={handleOpenIssue}
                        onRetry={() => void projectsWorkItemsQuery.refetch()}
                        profiles={profiles}
                        viewMode={viewMode}
                      />
                    ) : filter === "projects" ? (
                      projectItems
                    ) : (
                      repositoryItems
                    )}
                  </div>
                </section>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
