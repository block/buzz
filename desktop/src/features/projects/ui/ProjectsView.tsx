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
import { buildProjectSelectionAgentContext } from "@/features/projects/lib/projectDetailAgentContext";
import type { ProjectSelectionItem } from "@/features/projects/lib/projectSelection";
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
import { ProjectsOverviewChatToggle } from "@/features/projects/ui/ProjectsOverviewChatToggle";
import {
  ProjectsOverviewContextSheet,
  ProjectsOverviewNarrowContextToggle,
} from "@/features/projects/ui/ProjectsOverviewContextSheet";
import {
  ProjectsActivityIntro,
  ProjectsOverviewContextPanel,
  ProjectsOverviewPanel,
} from "@/features/projects/ui/ProjectsOverviewPanel";
import { ProjectContextRail } from "@/features/projects/ui/ProjectContextRail";
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
import { ProjectAgentChatPanel } from "@/features/projects/ui/ProjectAgentChatPanel";
import { ProjectsIssuesList } from "@/features/projects/ui/ProjectsIssuesList";
import { ProjectsWorkspaceChrome } from "@/features/projects/ui/ProjectDetailChrome";
import { ProjectsPullRequestsList } from "@/features/projects/ui/ProjectsPullRequestsList";
import { ProjectsWorkItemsLoadNotice } from "@/features/projects/ui/ProjectsWorkItemsLoadNotice";
import { ProjectsListHeaderBar } from "@/features/projects/ui/ProjectsListHeaderBar";
import { ProjectSectionHeader } from "@/features/projects/ui/ProjectSectionHeader";
import { PROJECT_COLUMN_HEADER_BACKDROP_CLASS } from "@/features/projects/ui/projectPanelStyles";
import { ProjectsToolbar } from "@/features/projects/ui/ProjectsToolbar";
import { ProjectSelectionProvider } from "@/features/projects/lib/useProjectSelection";
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
import { useProjectsScrollIndicator } from "@/features/projects/ui/useProjectsScrollIndicator";
import {
  PROJECT_CONTEXT_PANEL_DEFAULT_WIDTH_PX,
  useProjectPanelWidths,
} from "@/features/projects/ui/useProjectPanelWidths";
import { useMediaBreakpoint } from "@/shared/hooks/use-mobile";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import { topChromeInset } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { useRelayOrigin } from "@/shared/lib/useRelayOrigin";
import { Button } from "@/shared/ui/button";
import { useOptionalSidebar } from "@/shared/ui/sidebar";
import { useProjectsOverviewAgentContext } from "./useProjectsOverviewAgentContext";

const MANY_PROJECTS_THRESHOLD = 12;
const PROJECTS_CONTEXT_POD_MIN_VIEWPORT_PX = 1024;

export function ProjectsView() {
  const { goProject } = useAppNavigation();
  const { activeCommunity } = useCommunities();
  const relayOrigin = useRelayOrigin();
  const sidebar = useOptionalSidebar();
  const { handleContentScroll, scrollIndicatorRef } =
    useProjectsScrollIndicator();
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
  const { activeRightPanelWidth: overviewAgentPanelWidth } =
    useProjectPanelWidths("chat");
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
  const {
    agentContext: selectionAgentContext,
    overviewContext: overviewAgentContext,
    setAgentContext: setSelectionAgentContext,
  } = useProjectsOverviewAgentContext({
    filter,
    issues: projectsWorkItemsQuery.data?.issues.items,
    projects,
    pullRequests: projectsWorkItemsQuery.data?.pullRequests.items,
    snapshots: repoSnapshotsQuery.data?.snapshots,
    visibleIssues,
    visibleProjects,
    visiblePullRequests,
    visibleRepositories,
  });
  const handleFilterChange = React.useCallback(
    (nextFilter: ProjectsFilter) => {
      if (
        nextFilter === "projects" &&
        (repositoryScope === "buzz" || repositoryScope === "linked")
      ) {
        setRepositoryScope("all");
        writeStoredRepositoryScope("all");
      }
      setSelectionAgentContext(null);
      setFilter(nextFilter);
      writeStoredFilter(nextFilter);
    },
    [repositoryScope, setSelectionAgentContext],
  );

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

  const contextPanelProps = {
    filter,
    issues:
      projectsWorkItemsQuery.data?.issues.items.map(({ issue }) => issue) ?? [],
    onChatWithAgent: (items: ProjectSelectionItem[]) =>
      setSelectionAgentContext(buildProjectSelectionAgentContext(items)),
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
      <ProjectsOverviewNarrowContextToggle
        onToggle={() =>
          isNarrowProjectsLayout
            ? setNarrowContextOpen((open) => !open)
            : setOverviewPanelOpen((open) => !open)
        }
        open={contextOpen}
        ref={contextToggleRef}
      />
    </>
  );

  return (
    <ProjectSelectionProvider
      onSelect={() => {
        if (!isNarrowProjectsLayout) setOverviewPanelOpen(true);
      }}
      resetKey={filter}
    >
      <div
        className={cn(
          "relative flex min-h-0 min-w-0 flex-1 flex-row overflow-hidden",
          !isNarrowProjectsLayout && "bg-sidebar pb-2 pr-2 pt-px",
          !isNarrowProjectsLayout && sidebar?.open === false && "pl-2",
        )}
        data-project-context-detached={
          isNarrowProjectsLayout ? undefined : "true"
        }
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
            !isNarrowProjectsLayout
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
          <div className="flex min-h-0 min-w-0 flex-1">
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
                      <Button
                        aria-label="Search everything"
                        className="h-7 w-7 shrink-0 rounded-full border border-border/55 bg-transparent text-muted-foreground shadow-none hover:border-border hover:bg-muted/25 hover:text-foreground focus-visible:border-border"
                        data-testid="projects-activity-search"
                        onClick={openAppSearch}
                        size="icon"
                        title="Search everything"
                        type="button"
                        variant="ghost"
                      >
                        <Search className="h-4 w-4" />
                      </Button>
                      <div className="min-w-0 flex-1">
                        <ProjectsToolbar
                          filter={filter}
                          onFilterChange={handleFilterChange}
                        />
                      </div>
                      <ProjectsOverviewChatToggle
                        active={selectionAgentContext !== null}
                        onToggle={() =>
                          setSelectionAgentContext((context) =>
                            context ? null : overviewAgentContext,
                          )
                        }
                        sectionTitle={projectsSectionTitle(filter)}
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
                          <section className="space-y-3">
                            {activityFeed}
                          </section>
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
            {selectionAgentContext && !isNarrowProjectsLayout ? (
              <ProjectAgentChatPanel
                canResetWidth={overviewAgentPanelWidth.canReset}
                context={selectionAgentContext}
                onClose={() => setSelectionAgentContext(null)}
                onResetWidth={overviewAgentPanelWidth.onResetWidth}
                onResizeStart={overviewAgentPanelWidth.onResizeStart}
                widthPx={overviewAgentPanelWidth.widthPx}
              />
            ) : null}
          </div>
        </div>
        <ProjectContextRail
          open={overviewDetached}
          panelWidthPx={PROJECT_CONTEXT_PANEL_DEFAULT_WIDTH_PX}
          testId="projects-overview-context-rail"
        >
          <aside
            aria-label="Project context"
            className="relative z-30 flex h-full flex-col overflow-hidden bg-transparent"
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
        </ProjectContextRail>
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
    </ProjectSelectionProvider>
  );
}
