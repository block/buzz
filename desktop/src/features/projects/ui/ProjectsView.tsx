import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import {
  type Project,
  type ProjectIssue,
  type ProjectPullRequest,
  useDeleteProjectMutation,
  useProjectActivitySummariesQuery,
  useProjectLocalRepositoriesQuery,
  useProjectsQuery,
  useProjectsWorkItemsQuery,
} from "@/features/projects/hooks";
import { useCreateProjectMutation } from "@/features/projects/useCreateProject";
import { useProjectsRepoSnapshotsQuery } from "@/features/projects/useProjectsRepoSnapshots";
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
import { ProjectsListScopeDropdown } from "@/features/projects/ui/ProjectsListScopeDropdown";
import { PROJECT_LIST_CONTAINER_CLASS } from "@/features/projects/ui/projectListRowStyles";
import {
  ProjectsToolbar,
  ProjectsViewModeToggle,
} from "@/features/projects/ui/ProjectsToolbar";
import { hasLocalCheckout } from "@/features/projects/lib/projectLocalRepos";
import {
  getProjectUpdatedAt,
  isProjectMine,
  isProjectOwnedByCurrentUser,
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
  uniqueRepositories,
  writeStoredFilter,
  writeStoredIssueScope,
  writeStoredPullRequestScope,
  writeStoredRepositoryScope,
  writeStoredSort,
  writeStoredViewMode,
} from "@/features/projects/lib/projectsViewHelpers";
import { useOpenProjectTerminal } from "@/features/projects/ui/useOpenProjectTerminal";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import { topChromeInset } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { PageHeader } from "@/shared/ui/PageHeader";

const MANY_PROJECTS_THRESHOLD = 12;
const REPOSITORY_SCOPE_OPTIONS: Array<{
  label: string;
  value: ProjectsRepositoryScope;
}> = [
  { label: "All", value: "all" },
  { label: "My Repositories", value: "mine" },
  { label: "Local", value: "local" },
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

export function ProjectsView() {
  const { goProject } = useAppNavigation();
  const { activeCommunity } = useCommunities();
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
    filter === "prs" || filter === "issues" ? [] : projects,
  );
  const [repositoryScope, setRepositoryScope] =
    React.useState<ProjectsRepositoryScope>(() => readStoredRepositoryScope());
  const [pullRequestScope, setPullRequestScope] =
    React.useState<ProjectsWorkItemScope>(() => readStoredPullRequestScope());
  const [issueScope, setIssueScope] = React.useState<ProjectsWorkItemScope>(
    () => readStoredIssueScope(),
  );
  const projectsWorkItemsQuery = useProjectsWorkItemsQuery(
    filter === "all" || filter === "prs" || filter === "issues" ? projects : [],
  );
  // One blobless clone per unique repository — only scan while the overview
  // header (filter === "all") is actually visible.
  const snapshotProjects = React.useMemo(
    () => (filter === "all" ? uniqueRepositories(projects) : []),
    [filter, projects],
  );
  const repoSnapshotsQuery = useProjectsRepoSnapshotsQuery(
    snapshotProjects,
    activeCommunity?.reposDir,
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
            projectPeople(
              project,
              activitySummariesQuery.data?.[project.repoAddress],
            ),
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

  const handleFilterChange = React.useCallback((nextFilter: ProjectsFilter) => {
    setFilter(nextFilter);
    writeStoredFilter(nextFilter);
  }, []);

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

  // Count projects with a checkout on this machine — matches what the
  // "Local" filter actually lists, not every directory in the repos folder.
  const localProjectCount = React.useMemo(
    () =>
      projects.filter((project) => hasLocalCheckout(project, localRepoNames))
        .length,
    [localRepoNames, projects],
  );

  const visibleProjects = React.useMemo(() => {
    // The PRs and Issues filters render dedicated lists
    // (visiblePullRequests / visibleIssues), not project cards.
    if (filter === "prs" || filter === "issues") {
      return [];
    }

    const sortedProjects = projects
      .filter((project) => {
        const summary = activitySummariesQuery.data?.[project.repoAddress];
        const people = projectPeople(project, summary);
        if (repositoryScope === "mine")
          return isProjectMine(project, currentPubkey);
        if (repositoryScope === "local")
          return hasLocalCheckout(project, localRepoNames);
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

  const handleOpenCommit = React.useCallback(
    (project: Project, commitHash: string) => {
      void goProject(project.id, { commitHash });
    },
    [goProject],
  );

  const handleOpenPullRequest = React.useCallback(
    (project: Project, pullRequest: ProjectPullRequest) => {
      void goProject(project.id, { pullRequestId: pullRequest.id });
    },
    [goProject],
  );

  const handleOpenIssue = React.useCallback(
    (project: Project, issue: ProjectIssue) => {
      void goProject(project.id, { issueId: issue.id });
    },
    [goProject],
  );

  const openTerminal = useOpenProjectTerminal(activeCommunity?.reposDir);
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

  const repositoryItems =
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
          const summary = activitySummariesQuery.data?.[project.repoAddress];
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
              summary={summary}
            />
          );
        })}
      </div>
    ) : (
      <div className={PROJECT_LIST_CONTAINER_CLASS}>
        {visibleProjects.map((project) => {
          const summary = activitySummariesQuery.data?.[project.repoAddress];
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
              summary={summary}
            />
          );
        })}
      </div>
    );

  const listControls = (
    <div className="flex flex-wrap items-center gap-2 sm:justify-end">
      <label className="flex items-center gap-2 text-xs text-muted-foreground">
        <span className="sr-only">Sort projects</span>
        <select
          className="h-8 rounded-md bg-transparent px-2 text-xs text-foreground outline-hidden hover:bg-muted/50 focus:ring-1 focus:ring-ring"
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
        snapshots={repoSnapshotsQuery.data}
      />
    </>
  );

  const createMenu = (
    <ProjectsCreateMenu
      onCreateIssue={() => setCreateIssueOpen(true)}
      onCreatePullRequest={() => setCreatePullRequestOpen(true)}
      onCreateRepository={() => setCreateProjectOpen(true)}
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
      <div className="h-full min-w-0 flex-1">
        <ProjectsToolbar filter={filter} onFilterChange={handleFilterChange} />
      </div>
      {createMenu}
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
      <CreateProjectDialog
        isCreating={createProjectMutation.isPending}
        onCreate={async (input) => {
          const project = await createProjectMutation.mutateAsync(input);
          toast.success(`Project "${project.name}" created.`);
          // Land on the list that actually shows the new project — the
          // Overview only surfaces the top few most-active repositories.
          handleRepositoryScopeChange("all");
          handleFilterChange("repositories");
        }}
        onOpenChange={setCreateProjectOpen}
        open={createProjectOpen}
      />
      {createPullRequestOpen ? (
        <CreatePullRequestDialog
          onCreated={async (createdProject, pullRequestId) => {
            await goProject(createdProject.id, { pullRequestId });
          }}
          onOpenChange={setCreatePullRequestOpen}
          open
          projects={projects}
          reposDir={activeCommunity?.reposDir}
        />
      ) : null}
      <CreateProjectIssueDialog
        onCreated={async (createdProject, issueId) => {
          await goProject(createdProject.id, { issueId });
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
                  localRepositoryCount={localProjectCount}
                  metadata={
                    <ProjectsOverviewRail
                      profiles={profiles}
                      projects={projects}
                      summaries={activitySummariesQuery.data}
                    />
                  }
                  onSelectSection={(section) => {
                    if (section === "local") {
                      handleRepositoryScopeChange("local");
                      handleFilterChange("repositories");
                      return;
                    }
                    handleFilterChange(section);
                  }}
                  projects={projects}
                  summaries={activitySummariesQuery.data}
                >
                  <section className="space-y-3">{activityFeed}</section>
                </ProjectsOverviewPanel>
              ) : (
                <section className="space-y-3">
                  <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
                    {filter === "prs" ? (
                      <ProjectsListScopeDropdown
                        label="Filter pull requests"
                        onChange={handlePullRequestScopeChange}
                        options={PULL_REQUEST_SCOPE_OPTIONS}
                        value={pullRequestScope}
                      />
                    ) : filter === "issues" ? (
                      <ProjectsListScopeDropdown
                        label="Filter issues"
                        onChange={handleIssueScopeChange}
                        options={ISSUE_SCOPE_OPTIONS}
                        value={issueScope}
                      />
                    ) : (
                      <ProjectsListScopeDropdown
                        label="Filter repositories"
                        onChange={handleRepositoryScopeChange}
                        options={REPOSITORY_SCOPE_OPTIONS}
                        value={repositoryScope}
                      />
                    )}
                    {listControls}
                  </div>
                  {filter === "prs" ? (
                    <ProjectsPullRequestsList
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
                      error={projectsWorkItemsQuery.error}
                      failedSections={
                        projectsWorkItemsQuery.data?.issues.failedSections ?? []
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
                  ) : (
                    repositoryItems
                  )}
                </section>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
