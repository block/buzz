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
  useProjectsIssuesQuery,
  useProjectsPullRequestsQuery,
  useProjectsQuery,
} from "@/features/projects/hooks";
import { useProjectsRepoSnapshotsQuery } from "@/features/projects/useProjectsRepoSnapshots";
import { ProjectsActivityFeed } from "@/features/projects/ui/ProjectsActivityFeed";
import {
  EmptyFilteredState,
  EmptyState,
  ProjectGridCard,
  ProjectListRow,
  ProjectRailRow,
} from "@/features/projects/ui/ProjectCards";
import { ProjectsIssuesList } from "@/features/projects/ui/ProjectsIssuesList";
import { ProjectsOverviewPanel } from "@/features/projects/ui/ProjectsOverviewPanel";
import { ProjectsOverviewRail } from "@/features/projects/ui/ProjectsOverviewRail";
import { ProjectsPullRequestsList } from "@/features/projects/ui/ProjectsPullRequestsList";
import { ProjectsAgentPromptPage } from "@/features/projects/ui/ProjectsAgentPromptPage";
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
import { useOpenProjectTerminal } from "@/features/projects/ui/useOpenProjectTerminal";
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
import { Button } from "@/shared/ui/button";

const MANY_PROJECTS_THRESHOLD = 12;

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
    filter === "issues" || filter === "all" ? projects : [],
  );
  // One blobless clone per unique repository — only scan while the overview
  // header (filter === "all") is actually visible.
  const snapshotProjects = React.useMemo(
    () => (filter === "all" ? uniqueRepositories(projects) : []),
    [filter, projects],
  );
  const repoSnapshotsQuery = useProjectsRepoSnapshotsQuery(
    snapshotProjects,
    activeWorkspace?.reposDir,
  );
  const [searchOpen, setSearchOpen] = React.useState(false);
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
            ...pullRequest.reviewers,
            ...pullRequest.approvals.map((approval) => approval.author),
            ...pullRequest.updates.map((update) => update.author),
            ...pullRequest.comments.map((comment) => comment.author),
          ]) ?? []),
          ...(projectIssuesQuery.data?.flatMap(({ issue }) => [
            issue.author,
            ...issue.recipients,
            ...issue.comments.map((comment) => comment.author),
          ]) ?? []),
        ].map(normalizePubkey),
      ),
    ],
    [
      activitySummariesQuery.data,
      projectIssuesQuery.data,
      projectPullRequestsQuery.data,
      projects,
    ],
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
    // Picking a tab exits the full-page search state.
    setSearchOpen(false);
  }, []);

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
        if (filter === "mine") return isProjectMine(project, currentPubkey);
        if (filter === "local")
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

  // Route by the canonical `owner:dtag` project ID — a bare dtag is
  // ambiguous across owners (forks can share the same dtag).
  const handleOpenProject = React.useCallback(
    (project: Project) => {
      void goProject(project.id);
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
      <div className="-mx-4 divide-y divide-border/60 border-y border-border/60 bg-card">
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

  const mostActiveProjects = [...projects]
    .sort((left, right) => {
      const leftSummary = activitySummariesQuery.data?.[left.repoAddress];
      const rightSummary = activitySummariesQuery.data?.[right.repoAddress];
      const activityDifference =
        (rightSummary?.activityCount ?? 0) - (leftSummary?.activityCount ?? 0);
      if (activityDifference !== 0) return activityDifference;
      return (
        (rightSummary?.updatedAt ?? right.createdAt) -
        (leftSummary?.updatedAt ?? left.createdAt)
      );
    })
    .slice(0, 3);

  const mostActiveRepositoryItems =
    mostActiveProjects.length === 0 ? (
      <p className="px-2 py-8 text-center text-sm text-muted-foreground">
        No repository activity yet
      </p>
    ) : (
      <div className="space-y-1">
        {mostActiveProjects.map((project) => {
          const summary = activitySummariesQuery.data?.[project.repoAddress];
          return (
            <ProjectRailRow
              key={project.id}
              onOpen={handleOpenProject}
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

  const activityFeed = (
    <ProjectsActivityFeed
      isLoading={
        repoSnapshotsQuery.isLoading ||
        projectPullRequestsQuery.isLoading ||
        projectIssuesQuery.isLoading
      }
      issues={projectIssuesQuery.data ?? []}
      onOpenIssue={handleOpenIssue}
      onOpenProject={handleOpenProject}
      onOpenPullRequest={handleOpenPullRequest}
      profiles={profiles}
      projects={projects}
      pullRequests={projectPullRequestsQuery.data ?? []}
      snapshots={repoSnapshotsQuery.data}
    />
  );

  const projectsToolbar = (
    <ProjectsToolbar
      filter={filter}
      onFilterChange={handleFilterChange}
      onSearchOpenChange={setSearchOpen}
      searchOpen={searchOpen}
    />
  );

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
        {projectsToolbar}
      </div>

      {searchOpen ? (
        <ProjectsAgentPromptPage
          onClose={() => setSearchOpen(false)}
          projects={projects}
          workspaceId={activeWorkspace?.id ?? null}
        />
      ) : (
        <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto px-4 pb-4">
          <div className="pt-[calc(var(--buzz-channel-content-top-padding,5.75rem)_+_1rem)]">
            {filter === "all" ? (
              <ProjectsOverviewPanel
                localRepositoryCount={localProjectCount}
                metadata={
                  <ProjectsOverviewRail
                    profiles={profiles}
                    projects={projects}
                    snapshots={repoSnapshotsQuery.data}
                    snapshotsLoading={repoSnapshotsQuery.isLoading}
                    summaries={activitySummariesQuery.data}
                  >
                    {mostActiveRepositoryItems}
                  </ProjectsOverviewRail>
                }
                onSelectSection={handleFilterChange}
                projects={projects}
                relayName={activeWorkspace?.name || "Relay"}
                summaries={activitySummariesQuery.data}
              >
                <section className="space-y-3">
                  <h3 className="text-sm font-semibold text-foreground">
                    Recent activity
                  </h3>
                  {activityFeed}
                </section>
              </ProjectsOverviewPanel>
            ) : (
              <section className="space-y-3">
                <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
                  <h3 className="text-sm font-semibold text-foreground">
                    {filter === "prs"
                      ? "Pull requests"
                      : filter === "issues"
                        ? "Issues"
                        : "Repositories"}
                  </h3>
                  {listControls}
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
                ) : (
                  repositoryItems
                )}
              </section>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
