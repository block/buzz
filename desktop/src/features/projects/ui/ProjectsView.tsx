import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { ownsAuthorAgent } from "@/features/profile/lib/identity";
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
import { isExplicitProject } from "@/features/projects/projectModels";
import { projectsWithWorkItemRepositories } from "@/features/projects/projectWorkItems";
import { useProjectsRepoSnapshotsQuery } from "@/features/projects/useProjectsRepoSnapshots";
import { buildProjectSelectionAgentContext } from "@/features/projects/lib/projectDetailAgentContext";
import { buildProjectsActivityDigest } from "@/features/projects/lib/projectsActivityDigest";
import { matchesProjectsSearch } from "@/features/projects/lib/projectsSearch";
import type { ProjectSelectionItem } from "@/features/projects/lib/projectSelection";
import {
  useMemberChannelIds,
  useRepositoryUnavailableReasonFor,
} from "@/features/projects/useRepositoryAccess";
import { projectRepoHostForProject } from "@/features/projects/lib/projectRepoHost";
import { ProjectsActivityFeed } from "@/features/projects/ui/ProjectsActivityFeed";
import { ProjectsChannelsList } from "@/features/projects/ui/ProjectsChannelsList";
import {
  ProjectsOverviewContextSheet,
  ProjectsOverviewNarrowContextToggle,
} from "@/features/projects/ui/ProjectsOverviewContextSheet";
import {
  ProjectsActivityIntro,
  ProjectsOverviewContextPanel,
  ProjectsOverviewPanel,
} from "@/features/projects/ui/ProjectsOverviewPanel";
import { ProjectsOverviewChromeActions } from "@/features/projects/ui/ProjectsOverviewChromeActions";
import { ProjectContextRail } from "@/features/projects/ui/ProjectContextRail";
import {
  projectsSectionIcon,
  projectsSectionTitle,
} from "@/features/projects/ui/projectsSectionMeta";
import { EmptyState } from "@/features/projects/ui/ProjectCards";
import {
  ProjectsOverviewProjectItems,
  ProjectsOverviewRepositoryItems,
} from "@/features/projects/ui/ProjectsOverviewItems";
import { ProjectCreationDialog } from "@/features/projects/ui/ProjectCreationDialog";
import { CreateProjectIssueDialog } from "@/features/projects/ui/CreateProjectIssueDialog";
import { CreatePullRequestDialog } from "@/features/projects/ui/CreatePullRequestDialog";
import { ProjectAgentChatPanel } from "@/features/projects/ui/ProjectAgentChatPanel";
import { ProjectsCategoryCreateDialogs } from "@/features/projects/ui/ProjectsCategoryCreateDialogs";
import { ProjectsIssuesList } from "@/features/projects/ui/ProjectsIssuesList";
import { ProjectsWorkspaceChrome } from "@/features/projects/ui/ProjectDetailChrome";
import { ProjectsPullRequestsList } from "@/features/projects/ui/ProjectsPullRequestsList";
import { ProjectsWorkItemsLoadNotice } from "@/features/projects/ui/ProjectsWorkItemsLoadNotice";
import { ProjectsListHeaderBar } from "@/features/projects/ui/ProjectsListHeaderBar";
import { ProjectsSectionSearch } from "@/features/projects/ui/ProjectsSectionSearch";
import { ProjectSectionHeader } from "@/features/projects/ui/ProjectSectionHeader";
import { PROJECT_COLUMN_HEADER_BACKDROP_CLASS } from "@/features/projects/ui/projectPanelStyles";
import { ProjectSelectionProvider } from "@/features/projects/lib/useProjectSelection";
import { hasLocalRepositoryCheckout } from "@/features/projects/lib/projectLocalRepos";
import {
  getProjectUpdatedAt,
  projectHasAgent,
  projectOwnerIsUser,
  projectPeople,
  type ProjectsFilter,
  type ProjectsSort,
  type ProjectsViewMode,
  readStoredFilter,
  readStoredSort,
  readStoredViewMode,
  writeStoredFilter,
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
import { useNow } from "@/shared/lib/useNow";
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
import {
  EMPTY_ITEMS,
  useContextWorkItems,
  useDeleteProjectHandler,
  useOpenProjectTerminalHandler,
} from "./projectsViewWorkItems";

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
  const managedAgentsQuery = useManagedAgentsQuery();
  const projectReadModels = projectsQuery.data ?? [];
  const projects = React.useMemo(
    () => projectReadModels.filter(isExplicitProject),
    [projectReadModels],
  );
  const localRepositoriesQuery = useProjectLocalRepositoriesQuery(
    activeCommunity?.reposDir,
  );
  const [filter, setFilter] = React.useState<ProjectsFilter>(() => {
    const storedFilter = readStoredFilter();
    return storedFilter === "mine" || storedFilter === "local"
      ? "repositories"
      : storedFilter;
  });
  const [searchQuery, setSearchQuery] = React.useState("");
  const [overviewPanelOpen, setOverviewPanelOpen] = React.useState(true);
  const [narrowContextOpen, setNarrowContextOpen] = React.useState(false);
  const contextToggleRef = React.useRef<HTMLButtonElement | null>(null);
  const selectionDrawerStateRef = React.useRef<{
    narrow: boolean;
    open: boolean;
  } | null>(null);
  const isNarrowProjectsLayout = useMediaBreakpoint(
    PROJECTS_CONTEXT_POD_MIN_VIEWPORT_PX,
  );
  const { activeRightPanelWidth: overviewAgentPanelWidth } =
    useProjectPanelWidths("chat");
  const activitySummariesQuery = useProjectActivitySummariesQuery(projects);
  const repositoryActivitySummariesQuery = useRepositoryActivitySummariesQuery(
    filter === "repositories" ? projectReadModels : [],
  );
  const workItemProjects = React.useMemo(
    () => projectsWithWorkItemRepositories(projectReadModels),
    [projectReadModels],
  );
  const projectsWorkItemsQuery = useProjectsWorkItemsQuery(workItemProjects);
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
  const [createChannelOpen, setCreateChannelOpen] = React.useState(false);
  const [createRepositoryOpen, setCreateRepositoryOpen] = React.useState(false);
  const [createIssueOpen, setCreateIssueOpen] = React.useState(false);
  const [createPullRequestOpen, setCreatePullRequestOpen] =
    React.useState(false);
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
  const activityDigestNow = useNow(600_000);
  const activityDigest = React.useMemo(
    () =>
      buildProjectsActivityDigest({
        issues: projectsWorkItemsQuery.data?.issues.items ?? [],
        nowSeconds: Math.floor(activityDigestNow / 1_000),
        projects,
        pullRequests: projectsWorkItemsQuery.data?.pullRequests.items ?? [],
        snapshots: repoSnapshotsQuery.data?.snapshots,
        summaries: activitySummariesQuery.data,
      }),
    [
      activityDigestNow,
      activitySummariesQuery.data,
      projects,
      projectsWorkItemsQuery.data,
      repoSnapshotsQuery.data?.snapshots,
    ],
  );
  const deleteProjectMutation = useDeleteProjectMutation();
  const currentPubkey = identityQuery.data?.pubkey;
  const managedAgentPubkeys = React.useMemo(
    () =>
      new Set(
        (managedAgentsQuery.data ?? []).map((agent) =>
          normalizePubkey(agent.pubkey),
        ),
      ),
    [managedAgentsQuery.data],
  );
  const editableProjects = React.useMemo(() => {
    if (!currentPubkey) return [];
    const viewer = normalizePubkey(currentPubkey);
    return projects.filter((project) => {
      const owner = normalizePubkey(project.owner);
      return (
        owner === viewer ||
        managedAgentPubkeys.has(owner) ||
        ownsAuthorAgent(profiles?.[owner], currentPubkey)
      );
    });
  }, [currentPubkey, managedAgentPubkeys, profiles, projects]);
  const ownerControlAgentPubkeyFor = React.useCallback(
    (project: Project) => {
      const owner = normalizePubkey(project.owner);
      if (
        owner === normalizePubkey(currentPubkey ?? "") ||
        managedAgentPubkeys.has(owner)
      ) {
        return undefined;
      }
      return ownsAuthorAgent(profiles?.[owner], currentPubkey)
        ? project.owner
        : undefined;
    },
    [currentPubkey, managedAgentPubkeys, profiles],
  );

  const handleViewModeChange = React.useCallback(
    (nextViewMode: ProjectsViewMode) => {
      setStoredViewMode(nextViewMode);
      writeStoredViewMode(nextViewMode);
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

  const visibleProjects = React.useMemo(() => {
    if (filter !== "projects" && filter !== "agents" && filter !== "users") {
      return [];
    }

    const sortedProjects = projects
      .filter((project) => {
        if (
          !matchesProjectsSearch(searchQuery, [
            project.name,
            project.description,
            ...project.repositories.flatMap((repository) => [
              repository.name,
              repository.description,
            ]),
          ])
        ) {
          return false;
        }
        const summary = activitySummariesQuery.data?.[project.id];
        const people = projectPeople(project, summary);
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
    filter,
    profiles,
    projects,
    searchQuery,
    sort,
  ]);

  const visibleRepositories = React.useMemo(() => {
    if (filter !== "repositories") return [];
    const repositories = [
      ...new Map(
        projectReadModels
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
      .filter(({ project, repository }) =>
        matchesProjectsSearch(searchQuery, [
          repository.name,
          repository.description,
          project.name,
        ]),
      )
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
    filter,
    projectReadModels,
    repositoryActivitySummariesQuery.data,
    searchQuery,
    sort,
  ]);

  const visiblePullRequests = React.useMemo(() => {
    const pullRequests = projectsWorkItemsQuery.data?.pullRequests.items ?? [];
    return pullRequests
      .filter(({ project, pullRequest, repository }) =>
        matchesProjectsSearch(searchQuery, [
          pullRequest.title,
          pullRequest.content,
          pullRequest.status,
          project.name,
          repository.name,
        ]),
      )
      .sort((left, right) => {
        if (sort === "name") {
          return left.pullRequest.title.localeCompare(right.pullRequest.title);
        }
        if (sort === "created") {
          return right.pullRequest.createdAt - left.pullRequest.createdAt;
        }
        return right.pullRequest.updatedAt - left.pullRequest.updatedAt;
      });
  }, [projectsWorkItemsQuery.data, searchQuery, sort]);

  const visibleIssues = React.useMemo(() => {
    const issues = projectsWorkItemsQuery.data?.issues.items ?? [];
    return issues
      .filter(({ issue, project, repository }) =>
        matchesProjectsSearch(searchQuery, [
          issue.title,
          issue.content,
          issue.status,
          project.name,
          repository.name,
        ]),
      )
      .sort((left, right) => {
        if (sort === "name") {
          return left.issue.title.localeCompare(right.issue.title);
        }
        if (sort === "created") {
          return right.issue.createdAt - left.issue.createdAt;
        }
        return right.issue.updatedAt - left.issue.updatedAt;
      });
  }, [projectsWorkItemsQuery.data, searchQuery, sort]);
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
      writeStoredFilter(nextFilter);
      // Tab content swaps mount hundreds of rows/cards at once; a transition
      // lets React keep the click responsive and paint the previous tab until
      // the new tree is ready instead of blocking the main thread.
      React.startTransition(() => {
        setSelectionAgentContext(null);
        setFilter(nextFilter);
      });
    },
    [setSelectionAgentContext],
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
  const handleOpenTerminal = useOpenProjectTerminalHandler(
    openTerminal,
    localRepoNames,
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

  const handleDeleteProject = useDeleteProjectHandler(
    deleteProjectMutation.mutateAsync,
  );

  const { contextIssues, contextPullRequests } = useContextWorkItems(
    projectsWorkItemsQuery.data,
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

  if (projectReadModels.length === 0) {
    return (
      <>
        <ProjectCreationDialog
          onOpenChange={setCreateProjectOpen}
          open={createProjectOpen}
        />
        <EmptyState onCreateProject={() => setCreateProjectOpen(true)} />
      </>
    );
  }

  const projectItems = (
    <ProjectsOverviewProjectItems
      currentPubkey={currentPubkey}
      deleteDisabled={deleteProjectMutation.isPending}
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
      currentPubkey={currentPubkey}
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
      onViewModeChange={handleViewModeChange}
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
        issues={projectsWorkItemsQuery.data?.issues.items ?? EMPTY_ITEMS}
        onOpenCommit={handleOpenCommit}
        onOpenIssue={handleOpenIssue}
        onOpenProject={handleOpenProject}
        onOpenPullRequest={handleOpenPullRequest}
        profiles={profiles}
        projects={projects}
        pullRequests={
          projectsWorkItemsQuery.data?.pullRequests.items ?? EMPTY_ITEMS
        }
        searchQuery={searchQuery}
        snapshots={repoSnapshotsQuery.data?.snapshots}
      />
    </>
  );

  const contextPanelProps = {
    canCreateTarget: editableProjects.length > 0,
    filter,
    issues: contextIssues,
    onAddChannel: () => setCreateChannelOpen(true),
    onAddRepository: () => setCreateRepositoryOpen(true),
    onChatWithAgent: (items: ProjectSelectionItem[]) =>
      setSelectionAgentContext(buildProjectSelectionAgentContext(items)),
    onCreateIssue: () => setCreateIssueOpen(true),
    onCreateProject: () => setCreateProjectOpen(true),
    onCreatePullRequest: () => setCreatePullRequestOpen(true),
    profiles,
    projectReadModels,
    projects,
    pullRequests: contextPullRequests,
    repositorySummaries: repositoryActivitySummariesQuery.data,
    summaries: activitySummariesQuery.data,
  };
  const contextOpen = isNarrowProjectsLayout
    ? narrowContextOpen
    : overviewPanelOpen;
  const overviewChatOpen =
    selectionAgentContext !== null && !isNarrowProjectsLayout;
  const overviewContextOpen = overviewPanelOpen && !isNarrowProjectsLayout;
  const overviewDetached = overviewContextOpen || overviewChatOpen;
  const chromeActions = isNarrowProjectsLayout ? (
    <ProjectsOverviewNarrowContextToggle
      onToggle={() => setNarrowContextOpen((open) => !open)}
      open={contextOpen}
      ref={contextToggleRef}
    />
  ) : (
    <ProjectsOverviewChromeActions
      chatOpen={overviewChatOpen}
      contextOpen={overviewPanelOpen}
      onToggleChat={() =>
        setSelectionAgentContext((context) =>
          context ? null : overviewAgentContext,
        )
      }
      onToggleContext={() => setOverviewPanelOpen((open) => !open)}
      sectionTitle={projectsSectionTitle(filter)}
    />
  );

  return (
    <ProjectSelectionProvider
      onClear={() => {
        const previous = selectionDrawerStateRef.current;
        selectionDrawerStateRef.current = null;
        if (!previous) return;
        if (previous.narrow) {
          setNarrowContextOpen(previous.open);
        } else {
          setOverviewPanelOpen(previous.open);
        }
      }}
      onSelect={() => {
        if (selectionDrawerStateRef.current) return;
        selectionDrawerStateRef.current = {
          narrow: isNarrowProjectsLayout,
          open: isNarrowProjectsLayout ? narrowContextOpen : overviewPanelOpen,
        };
        if (isNarrowProjectsLayout) {
          setNarrowContextOpen(true);
        } else {
          setOverviewPanelOpen(true);
        }
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
          <ProjectCreationDialog
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
          <ProjectsCategoryCreateDialogs
            channelOpen={createChannelOpen}
            editableProjects={editableProjects}
            onChannelOpenChange={setCreateChannelOpen}
            onRepositoryOpenChange={setCreateRepositoryOpen}
            ownerControlAgentPubkeyFor={ownerControlAgentPubkeyFor}
            repositoryOpen={createRepositoryOpen}
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
                      <ProjectsSectionSearch
                        filter={filter}
                        onFilterChange={handleFilterChange}
                        onQueryChange={setSearchQuery}
                        onSortChange={handleSortChange}
                        sort={sort}
                      />
                    </div>
                    <div
                      className={
                        filter === "all" ? "mx-auto w-full max-w-6xl" : "w-full"
                      }
                    >
                      {filter === "all" ? (
                        <ProjectsOverviewPanel>
                          <ProjectsActivityIntro digest={activityDigest} />
                          <section className="space-y-3">
                            {activityFeed}
                          </section>
                        </ProjectsOverviewPanel>
                      ) : (
                        <>
                          <ProjectSectionHeader
                            className="mb-2 rounded-md bg-muted/40"
                            icon={projectsSectionIcon(filter)}
                            testId="projects-page-header"
                            title={projectsSectionTitle(filter)}
                            trailing={
                              filter === "channels" ? undefined : listHeaderBar
                            }
                          />
                          <section>
                            <div className="space-y-3">
                              {filter === "prs" ? (
                                <ProjectsPullRequestsList
                                  embedded={viewMode === "list"}
                                  emptyMessage={
                                    searchQuery.trim()
                                      ? "No matching reviews"
                                      : undefined
                                  }
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
                                  emptyMessage={
                                    searchQuery.trim()
                                      ? "No matching tasks"
                                      : undefined
                                  }
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
                                <ProjectsChannelsList
                                  projects={projectReadModels}
                                  searchQuery={searchQuery}
                                />
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
        </div>
        <ProjectContextRail
          open={overviewChatOpen}
          panelWidthPx={overviewAgentPanelWidth.widthPx}
          resizing={overviewAgentPanelWidth.isResizing}
          testId="projects-overview-agent-rail"
        >
          {selectionAgentContext ? (
            <ProjectAgentChatPanel
              canResetWidth={overviewAgentPanelWidth.canReset}
              constrainToAvailableSpace={false}
              context={selectionAgentContext}
              detached
              onClose={() => setSelectionAgentContext(null)}
              onResetWidth={overviewAgentPanelWidth.onResetWidth}
              onResizeStart={overviewAgentPanelWidth.onResizeStart}
              widthPx={overviewAgentPanelWidth.widthPx}
            />
          ) : null}
        </ProjectContextRail>
        <ProjectContextRail
          open={overviewContextOpen}
          panelWidthPx={PROJECT_CONTEXT_PANEL_DEFAULT_WIDTH_PX}
          rounded={false}
          testId="projects-overview-context-rail"
        >
          <aside
            aria-label="Project context"
            className="relative z-30 flex h-full flex-col overflow-hidden bg-sidebar text-sidebar-foreground"
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
