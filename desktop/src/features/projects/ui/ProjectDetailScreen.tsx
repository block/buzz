import { ArrowLeft, FolderGit2 } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useTerminalContextOverride } from "@/app/TerminalContextOverrideContext";
import {
  type Project,
  type Repository,
  useProjectQuery,
  useProjectIssuesQuery,
  useProjectLocalRepoDiffQuery,
  useProjectLocalRepoSnapshotQuery,
  useProjectRepoDiffQuery,
  useProjectPullRequestsQuery,
  useProjectRepoSnapshotQuery,
  useProjectsQuery,
  useRepoStateQuery,
} from "@/features/projects/hooks";
import {
  useCloneProjectRepositoryMutation,
  useProjectRepoSyncStatusQuery,
  usePullProjectLocalRepositoryMutation,
  usePushProjectLocalRepositoryMutation,
} from "@/features/projects/repoSyncHooks";
import { useProjectBranchActions } from "@/features/projects/branchMutations";
import { useOptimisticProjectBranches } from "@/features/projects/useOptimisticProjectBranches";
import { useProjectRepositoryRefSelection } from "@/features/projects/useProjectRepositoryRefSelection";
import { useUpdateProjectPullRequestMutation } from "@/features/projects/pullRequestMutations";
import { useCreateProjectIssueMutation } from "@/features/projects/issueMutations";
import { UserProfilePanel } from "@/features/profile/ui/UserProfilePanel";
import { ProfilePanelProvider } from "@/shared/context/ProfilePanelContext";
import { useHistorySearchState } from "@/shared/hooks/useHistorySearchState";
import { Button } from "@/shared/ui/button";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import { useCommunities } from "@/features/communities/useCommunities";
import { useProjectCommitDiffQuery } from "@/features/projects/useProjectCommitDiff";
import {
  projectBranchCreationReason,
  projectBranchManagementState,
  projectBranchOptionsFromSync,
  resolveProjectDefaultBranch,
} from "@/features/projects/lib/projectBranches";
import { normalizeRepositoryUrl } from "@/features/projects/lib/projectsViewHelpers";
import {
  shareTabForWorkspaceTab,
  workspaceTabForShareTab,
} from "@/features/projects/lib/projectShareLinks";
import {
  buildProjectDetailAgentContext,
  type ProjectDetailAgentContext,
} from "@/features/projects/lib/projectDetailAgentContext";
import {
  projectRepoUnavailablePresentation,
  projectRepoUnavailableReason,
  refineRepoUnavailableReason,
} from "@/features/projects/lib/projectRepoAvailability";
import { selectProjectRepository } from "@/features/projects/projectModels";
import { useMemberChannelIds } from "@/features/projects/useRepositoryAccess";
import { KIND_REPO_ANNOUNCEMENT } from "@/shared/constants/kinds";
import type { EntityLinkTab } from "@/shared/lib/entityLink";
import { useProjectRepoPresentation } from "@/features/projects/useProjectRepoHost";
import { WorkspaceTabs } from "./ProjectWorkspaceTabs";
import type { RepoSourceHeaderControls } from "./ProjectRepositorySource";
import { showProjectCloneErrorToast } from "./projectGitErrorToast";
import { projectTerminalLabel } from "./useOpenProjectTerminal";
import type { CreateIssueDialogInput } from "./CreateIssueDialog";
import { ProjectBranchActionDialogs } from "./ProjectBranchActionDialogs";
import { ProjectDetailChrome } from "./ProjectDetailChrome";
import { ProjectConversationPanelController } from "./ProjectConversationPanelContext";
import { ProjectDetailRightPanel } from "./ProjectDetailRightPanel";
import { ProjectRightPanelControls } from "./ProjectRightPanelControls";
import { UnavailableProjectRepositories } from "./UnavailableProjectRepositories";
import { buildProjectDetailCrumbs } from "./useProjectDetailCrumbs";
import { useProjectDetailPeople } from "./useProjectDetailPeople";
import { useProjectProfilePanel } from "./useProjectProfilePanel";
import { useProjectRepositoryPanel } from "./useProjectRepositoryPanel";
import { useRepositoryFileContentSource } from "./useRepositoryFileContentSource";
import { useProjectPanelWidths } from "./useProjectPanelWidths";
import { useProjectRepositoryOpenActions } from "./useProjectRepositoryOpenActions";
import {
  PROJECT_REPOSITORY_SEARCH_KEYS,
  type ProjectDetailScreenProps,
  pushPullTitle,
  snapshotHasContent,
} from "./projectDetailHelpers";

export function ProjectDetailScreen(props: ProjectDetailScreenProps) {
  const {
    commitHash,
    entityNavigationId,
    projectId,
    pullRequestId,
    issueId,
    repositoryId,
    tab,
  } = props;
  const { goProject, goProjects } = useAppNavigation();
  const { activeCommunity } = useCommunities();
  const projectQuery = useProjectQuery(projectId);
  const projectsQuery = useProjectsQuery();
  const project = projectQuery.data;
  const routeRepositoryId: string | undefined = React.useMemo(() => {
    if (repositoryId) return repositoryId;
    const kindStr = `${String(KIND_REPO_ANNOUNCEMENT)}:`;
    if (!projectId.startsWith(kindStr)) return undefined;
    return projectId.slice(kindStr.length);
  }, [projectId, repositoryId]);
  const repository = selectProjectRepository(project, routeRepositoryId);
  const repoRemote = useProjectRepoPresentation(repository);
  const { applyPatch: applyRepositorySearch } = useHistorySearchState(
    PROJECT_REPOSITORY_SEARCH_KEYS,
  );
  const repoStateQuery = useRepoStateQuery(repository);
  const pullRequestsQuery = useProjectPullRequestsQuery(repository);
  const defaultBranch = repository
    ? resolveProjectDefaultBranch(repository.defaultBranch, repoStateQuery.data)
    : null;
  const { branchOptions, forgetBranch, managedBranches, rememberBranch } =
    useOptimisticProjectBranches({
      defaultBranch,
      observedBranches: repoStateQuery.data?.branches ?? [],
      projectId: repository?.id ?? projectId,
      referencedBranches:
        pullRequestsQuery.data?.map(
          (pullRequest) => pullRequest.branchName ?? null,
        ) ?? [],
    });
  const { activeBranch, selectBranch, selectedTag, selectTag } =
    useProjectRepositoryRefSelection({
      branchOptions,
      defaultBranch,
      projectAvailable: Boolean(repository),
      projectPending: projectQuery.isPending,
      repositoryId: repository?.id ?? null,
      tags: repoStateQuery.data?.tags ?? [],
    });
  const activeTag =
    repoStateQuery.data?.tags.find((tag) => tag.name === selectedTag) ?? null;
  const [selectedPullRequestId, setSelectedPullRequestId] = React.useState<
    string | null
  >(pullRequestId ?? null);
  const [selectedIssueId, setSelectedIssueId] = React.useState<string | null>(
    issueId ?? null,
  );
  const [createIssueRequestKey, setCreateIssueRequestKey] = React.useState(0);
  // biome-ignore lint/correctness/useExhaustiveDependencies: the transient request ID deliberately reapplies an unchanged entity selection.
  React.useEffect(() => {
    setSelectedPullRequestId(pullRequestId ?? null);
    setSelectedIssueId(issueId ?? null);
  }, [entityNavigationId, issueId, pullRequestId]);
  const [selectedCommitHash, setSelectedCommitHash] = React.useState<
    string | null
  >(commitHash ?? null);
  React.useEffect(
    () => setSelectedCommitHash(commitHash ?? null),
    [commitHash],
  );
  const [tabsResetKey, setTabsResetKey] = React.useState(0);
  const [requestedTab, setRequestedTab] = React.useState<
    EntityLinkTab | undefined
  >(tab);
  // biome-ignore lint/correctness/useExhaustiveDependencies: the transient request ID deliberately reapplies an unchanged share-link tab.
  React.useEffect(() => setRequestedTab(tab), [entityNavigationId, tab]);
  const [activeTab, setActiveTab] = React.useState("overview");
  const [filesContext, setFilesContext] =
    React.useState<ProjectDetailAgentContext["file"]>(null);
  const handleSelectedPullRequestIdChange = React.useCallback(
    (id: string | null) => {
      setSelectedPullRequestId(id);
      if (id) setSelectedCommitHash(null);
    },
    [],
  );
  const handleSelectedIssueIdChange = React.useCallback((id: string | null) => {
    setSelectedIssueId(id);
    if (id) setSelectedCommitHash(null);
  }, []);
  const handleSelectedCommitHashChange = React.useCallback(
    (hash: string | null) => {
      setSelectedCommitHash(hash);
      if (hash) {
        setSelectedPullRequestId(null);
        setSelectedIssueId(null);
      }
    },
    [],
  );
  const issuesQuery = useProjectIssuesQuery(repository);
  const selectedBranchPullRequest = React.useMemo(() => {
    const projectRepositories = new Set(
      (repository?.cloneUrls ?? []).map(normalizeRepositoryUrl),
    );
    const matches =
      pullRequestsQuery.data?.filter(
        (pullRequest) =>
          pullRequest.branchName === activeBranch &&
          pullRequest.cloneUrls.some((cloneUrl) =>
            projectRepositories.has(normalizeRepositoryUrl(cloneUrl)),
          ),
      ) ?? [];
    return matches.length === 1 ? matches[0] : null;
  }, [activeBranch, pullRequestsQuery.data, repository?.cloneUrls]);
  const openBranchPullRequest =
    selectedBranchPullRequest?.status === "Open" ||
    selectedBranchPullRequest?.status === "Draft"
      ? selectedBranchPullRequest
      : null;
  const activeRepoPullRequest =
    pullRequestsQuery.data?.find((item) => item.id === selectedPullRequestId) ??
    selectedBranchPullRequest;
  const [repoSource, setRepoSource] = React.useState<"remote" | "local">(
    "remote",
  );
  const repositoryPanel = useProjectRepositoryPanel();
  const repoSnapshotQuery = useProjectRepoSnapshotQuery(
    repository,
    activeBranch,
    selectedTag ? null : selectedBranchPullRequest,
    activeTag,
    repoRemote.host.kind === "buzz",
  );
  const memberChannelIds = useMemberChannelIds();
  const remoteUnavailableReason =
    repoRemote.host.kind === "buzz" &&
    !repoSnapshotQuery.isLoading &&
    (!repoSnapshotQuery.data || repoSnapshotQuery.error)
      ? refineRepoUnavailableReason({
          reason: projectRepoUnavailableReason(repoSnapshotQuery.error),
          repositoryChannelId: repository?.channelId,
          memberChannelIds,
        })
      : undefined;
  const repoDiffQuery = useProjectRepoDiffQuery(
    repository,
    activeBranch,
    activeRepoPullRequest,
    repoSource === "remote",
  );
  const localRepoDiffQuery = useProjectLocalRepoDiffQuery(
    repository,
    activeCommunity?.reposDir,
    activeBranch,
    activeRepoPullRequest,
    repoSource === "local" && Boolean(activeRepoPullRequest),
  );
  const commitDiffQuery = useProjectCommitDiffQuery(
    repository,
    selectedCommitHash,
    repoSource,
    activeCommunity?.reposDir,
  );
  const localRepoSnapshotQuery = useProjectLocalRepoSnapshotQuery(
    repository,
    activeCommunity?.reposDir,
    activeBranch,
  );
  const repoSyncStatusQuery = useProjectRepoSyncStatusQuery(
    repository,
    activeCommunity?.reposDir,
    activeBranch,
  );
  const pushLocalRepoMutation = usePushProjectLocalRepositoryMutation(
    repository,
    activeCommunity?.reposDir,
    activeBranch,
    openBranchPullRequest,
  );
  const pullLocalRepoMutation = usePullProjectLocalRepositoryMutation(
    repository,
    activeCommunity?.reposDir,
    activeBranch,
  );
  const cloneRepoMutation = useCloneProjectRepositoryMutation(
    repository,
    activeCommunity?.reposDir,
  );
  const createIssueMutation = useCreateProjectIssueMutation(repository);
  const updatePullRequestMutation = useUpdateProjectPullRequestMutation(
    repository,
    openBranchPullRequest,
  );
  const hasLocalCheckout = Boolean(
    localRepoSnapshotQuery.data || repoSyncStatusQuery.data?.localPath,
  );
  const hasRemoteSnapshot = snapshotHasContent(repoSnapshotQuery.data);
  const displayedRepoDiff =
    repoSource === "local" ? localRepoDiffQuery.data : repoDiffQuery.data;
  const displayedRepoDiffError =
    repoSource === "local" ? localRepoDiffQuery.error : repoDiffQuery.error;
  const displayedRepoDiffLoading =
    repoSource === "local"
      ? localRepoDiffQuery.isLoading
      : repoDiffQuery.isLoading;
  const branchOptionsWithLocal = projectBranchOptionsFromSync(
    branchOptions,
    repoSyncStatusQuery.data,
  );
  const { activeBranchCommit, activeRemoteBranch, deleteBranchReason } =
    projectBranchManagementState({
      activeBranch,
      branches: managedBranches,
      defaultBranch,
      hasOpenPullRequest: (pullRequestsQuery.data ?? []).some(
        (pullRequest) =>
          pullRequest.branchName === activeBranch &&
          (pullRequest.status === "Open" || pullRequest.status === "Draft"),
      ),
      remoteBranch: repoSyncStatusQuery.data?.remoteBranch,
      remoteHead: repoSyncStatusQuery.data?.remoteHead,
      snapshotCommit: repoSnapshotQuery.data?.latestCommit?.hash,
    });
  const handleBranchChange = React.useCallback(
    (branch: string | null) => {
      selectBranch(branch);
      if (
        branch &&
        repoSource === "local" &&
        branch !== repoSyncStatusQuery.data?.localBranch
      ) {
        setRepoSource("remote");
      }
    },
    [repoSource, repoSyncStatusQuery.data?.localBranch, selectBranch],
  );
  const handleTagChange = React.useCallback(
    (tag: string) => {
      selectTag(tag);
      setRepoSource("remote");
    },
    [selectTag],
  );
  const branchActions = useProjectBranchActions({
    activeBranch,
    activeBranchCommit,
    activeRemoteBranch,
    defaultBranch,
    deleteBranchReason,
    forgetBranch,
    project: repository,
    refetchRepoState: repoStateQuery.refetch,
    rememberBranch,
    selectBranch: handleBranchChange,
  });
  const createBranchReason = projectBranchCreationReason({
    activeBranch,
    activeBranchCommit,
    localHead: repoSyncStatusQuery.data?.localHead,
  });
  const handleFetchRepo = React.useCallback(async () => {
    const results = await Promise.all([
      repoSnapshotQuery.refetch(),
      repoStateQuery.refetch(),
      repoSyncStatusQuery.refetch(),
    ]);
    const error = results.find((result) => result.error)?.error;
    if (error) {
      const reason = refineRepoUnavailableReason({
        reason: projectRepoUnavailableReason(error),
        repositoryChannelId: repository?.channelId,
        memberChannelIds,
      });
      const presentation = projectRepoUnavailablePresentation(reason);
      toast.error(presentation.title, {
        description: presentation.description,
      });
      return;
    }
    toast.success("Remote state refreshed.");
  }, [
    memberChannelIds,
    repoSnapshotQuery,
    repoStateQuery,
    repoSyncStatusQuery,
    repository?.channelId,
  ]);
  const cloneBlockedByRemote =
    remoteUnavailableReason !== undefined &&
    remoteUnavailableReason !== "ref" &&
    remoteUnavailableReason !== "unknown";
  const filesSourceControls: RepoSourceHeaderControls = {
    branch: activeBranch ?? "",
    branchOptions: branchOptionsWithLocal,
    selectedTag,
    tagOptions: repoStateQuery.data?.tags ?? [],
    onBranchChange: handleBranchChange,
    onTagChange: handleTagChange,
    onCreateBranch: () => branchActions.setCreateOpen(true),
    createBranchDisabled: branchActions.createPending || !activeBranchCommit,
    createBranchTitle: createBranchReason ?? "Create a remote branch",
    onDeleteBranch: () => branchActions.setDeleteOpen(true),
    deleteBranchDisabled:
      branchActions.deletePending || Boolean(deleteBranchReason),
    deleteBranchTitle: deleteBranchReason ?? "Delete this remote branch",
    source: selectedTag ? "remote" : repoSource,
    onSourceChange: setRepoSource,
    localDisabled:
      Boolean(selectedTag) ||
      (!repoSyncStatusQuery.data?.localPath &&
        !localRepoSnapshotQuery.data &&
        !localRepoSnapshotQuery.isLoading),
    localLabel: localRepoSnapshotQuery.isLoading
      ? "Local checking"
      : repoSyncStatusQuery.data?.localPath || localRepoSnapshotQuery.data
        ? "Local"
        : "Local missing",
    ...repoRemote.controls,
    remoteUnavailableReason,
    onAskForAccess: () => {
      repositoryPanel.setMode("chat");
      repositoryPanel.expand();
    },
    onCloneLocal:
      !selectedTag &&
      !cloneBlockedByRemote &&
      repository?.cloneUrls[0] &&
      repoRemote.canCloneLocally
        ? () => {
            void handleCloneRepo();
          }
        : undefined,
    clonePending: cloneRepoMutation.isPending,
    canPush: !selectedTag && (repoSyncStatusQuery.data?.canPush ?? false),
    onPush: selectedTag
      ? undefined
      : () => {
          void handlePushLocalRepo();
        },
    pushDisabled:
      pushLocalRepoMutation.isPending || !repoSyncStatusQuery.data?.canPush,
    pushPending: pushLocalRepoMutation.isPending,
    pushTitle:
      repoSyncStatusQuery.data?.pushBlockReason ??
      pushPullTitle("Push", repoSyncStatusQuery.data?.aheadCount, "local"),
    canPull: !selectedTag && (repoSyncStatusQuery.data?.canPull ?? false),
    onPull: selectedTag
      ? undefined
      : () => {
          void handlePullLocalRepo();
        },
    pullDisabled:
      pullLocalRepoMutation.isPending || !repoSyncStatusQuery.data?.canPull,
    pullPending: pullLocalRepoMutation.isPending,
    pullTitle:
      repoSyncStatusQuery.data?.pullBlockReason ??
      pushPullTitle("Pull", repoSyncStatusQuery.data?.behindCount, "remote"),
    aheadCount: repoSyncStatusQuery.data?.aheadCount ?? null,
    behindCount: repoSyncStatusQuery.data?.behindCount ?? null,
    onFetch: () => {
      void handleFetchRepo();
    },
    fetchPending:
      repoSnapshotQuery.isFetching ||
      repoStateQuery.isFetching ||
      repoSyncStatusQuery.isFetching,
    fetchTitle:
      repoSyncStatusQuery.data?.pullBlockReason ?? "Check for remote changes",
  };
  const fileContentSource = useRepositoryFileContentSource({
    activeBranch,
    activeTag,
    pullRequest: selectedBranchPullRequest,
    repository,
    reposDir: activeCommunity?.reposDir,
    selectedTag,
    source: repoSource,
  });
  const projectPending = projectQuery.isPending;
  React.useEffect(() => {
    if (!repository) {
      if (projectPending) return;
      setSelectedPullRequestId(null);
      setSelectedIssueId(null);
      setSelectedCommitHash(null);
    }
  }, [projectPending, repository]);
  React.useEffect(() => {
    if (selectedTag) {
      if (repoSource !== "remote") setRepoSource("remote");
      return;
    }
    if (repoSource === "local" && !hasLocalCheckout) {
      setRepoSource("remote");
      return;
    }
    if (repoSource === "remote" && !hasRemoteSnapshot && hasLocalCheckout) {
      setRepoSource("local");
    }
  }, [hasLocalCheckout, hasRemoteSnapshot, repoSource, selectedTag]);
  const {
    contributorActivityCounts,
    contributorPubkeys,
    identityPubkey,
    profiles,
    viewerGitIdentity,
  } = useProjectDetailPeople({
    issues: issuesQuery.data ?? [],
    pullRequests: pullRequestsQuery.data ?? [],
    repository,
  });
  const {
    handleCloseProfilePanel,
    handleOpenDm,
    handleOpenProfilePanel,
    handleProfilePanelTabChange,
    handleProfilePanelViewChange,
    profilePanelPubkey,
    profilePanelTab,
    profilePanelView,
  } = useProjectProfilePanel();
  const { activeRightPanelWidth, threadPanelWidth } = useProjectPanelWidths(
    repositoryPanel.mode,
  );
  const handlePushLocalRepo = React.useCallback(async () => {
    try {
      const result = await pushLocalRepoMutation.mutateAsync();
      if (result.pullRequestUpdate.status === "failed") {
        toast.warning(result.message, {
          description: result.pullRequestUpdate.error,
        });
      } else {
        toast.success(
          result.pullRequestUpdate.status === "updated"
            ? `${result.message} Review updated.`
            : result.message,
        );
      }
      await Promise.all([
        repoSnapshotQuery.refetch(),
        localRepoSnapshotQuery.refetch(),
        repoSyncStatusQuery.refetch(),
        repoStateQuery.refetch(),
      ]);
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Failed to push repository",
      );
    }
  }, [
    localRepoSnapshotQuery,
    pushLocalRepoMutation,
    repoSnapshotQuery,
    repoStateQuery,
    repoSyncStatusQuery,
  ]);
  const handleCloneRepo = React.useCallback(async () => {
    try {
      const result = await cloneRepoMutation.mutateAsync();
      toast.success(result.message);
      setRepoSource("local");
    } catch (error) {
      const unavailableReason = refineRepoUnavailableReason({
        reason: projectRepoUnavailableReason(error),
        repositoryChannelId: repository?.channelId,
        memberChannelIds,
      });
      showProjectCloneErrorToast(
        error,
        repository?.cloneUrls[0],
        unavailableReason,
      );
    }
  }, [
    cloneRepoMutation,
    memberChannelIds,
    repository?.channelId,
    repository?.cloneUrls,
  ]);
  const handlePullRequestCreated = React.useCallback(
    async (
      createdProject: Project,
      createdRepository: Repository,
      pullRequestId: string,
    ) => {
      if (createdProject.id !== projectId) {
        await goProject(createdProject.id, {
          pullRequestId,
          repositoryId: createdRepository.id,
        });
        return;
      }
      if (createdRepository.id === repository?.id) {
        await pullRequestsQuery.refetch();
      } else {
        applyRepositorySearch({ repositoryId: createdRepository.id });
      }
      setSelectedPullRequestId(pullRequestId);
    },
    [
      applyRepositorySearch,
      goProject,
      projectId,
      pullRequestsQuery,
      repository?.id,
    ],
  );
  const handleCreateIssue = React.useCallback(
    async (input: CreateIssueDialogInput) => {
      const issueId = await createIssueMutation.mutateAsync(input);
      toast.success("Task created.");
      await issuesQuery.refetch();
      setSelectedIssueId(issueId);
    },
    [createIssueMutation, issuesQuery],
  );
  const handleUpdatePullRequest = React.useCallback(async () => {
    const commit = repoSyncStatusQuery.data?.remoteHead;
    if (!commit) return;
    try {
      const updated = await updatePullRequestMutation.mutateAsync({
        commit,
        mergeBase: repoSyncStatusQuery.data?.mergeBase ?? null,
      });
      toast.success(updated ? "Review updated." : "Review is already current.");
      await pullRequestsQuery.refetch();
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Failed to update review",
      );
    }
  }, [
    pullRequestsQuery,
    repoSyncStatusQuery.data?.mergeBase,
    repoSyncStatusQuery.data?.remoteHead,
    updatePullRequestMutation,
  ]);
  const handlePullLocalRepo = React.useCallback(async () => {
    try {
      const result = await pullLocalRepoMutation.mutateAsync();
      toast.success(result.message);
      await Promise.all([
        repoSnapshotQuery.refetch(),
        localRepoSnapshotQuery.refetch(),
        repoSyncStatusQuery.refetch(),
        repoStateQuery.refetch(),
      ]);
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Failed to pull repository",
      );
    }
  }, [
    localRepoSnapshotQuery,
    pullLocalRepoMutation,
    repoSnapshotQuery,
    repoStateQuery,
    repoSyncStatusQuery,
  ]);
  const localRepositoryPath =
    repoSyncStatusQuery.data?.localPath ??
    localRepoSnapshotQuery.data?.path ??
    null;
  const {
    handleOpenLocalRepository,
    handleOpenMergeRecoveryTerminal,
    handleOpenTerminal,
  } = useProjectRepositoryOpenActions({
    activeBranch,
    hasLocalCheckout,
    localRepositoryPath,
    repository,
    reposDir: activeCommunity?.reposDir,
  });
  const projectTerminalContext = React.useMemo(() => {
    const channelId = repository?.channelId ?? project?.projectChannelId;
    if (!channelId) return null;
    return {
      channelId,
      channelName: repository?.name ?? project?.name ?? "Project",
    };
  }, [
    project?.name,
    project?.projectChannelId,
    repository?.channelId,
    repository?.name,
  ]);
  useTerminalContextOverride(projectTerminalContext);

  if (projectQuery.isLoading) {
    return <ViewLoadingFallback kind="projects" />;
  }
  if (projectQuery.isError) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 px-4 py-16 text-center">
        <FolderGit2 className="h-10 w-10 text-muted-foreground/40" />
        <p className="text-sm text-red-400">Failed to load project</p>
        <div className="flex items-center gap-2">
          <Button
            onClick={() => void projectQuery.refetch()}
            size="sm"
            variant="outline"
          >
            Retry
          </Button>
          <Button
            onClick={() => {
              void goProjects();
            }}
            size="sm"
            variant="ghost"
          >
            <ArrowLeft className="mr-1.5 h-4 w-4" />
            Back to Projects
          </Button>
        </div>
      </div>
    );
  }
  if (!project) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 px-4 py-16 text-center">
        <FolderGit2 className="h-10 w-10 text-muted-foreground/40" />
        <p className="text-sm text-muted-foreground">
          This project could not be found.
        </p>
        <Button
          onClick={() => {
            void goProjects();
          }}
          size="sm"
          variant="outline"
        >
          <ArrowLeft className="mr-1.5 h-4 w-4" />
          Back to Projects
        </Button>
      </div>
    );
  }
  if (!repository) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 px-4 py-16 text-center">
        <FolderGit2 className="h-10 w-10 text-muted-foreground/40" />
        <p className="text-sm font-medium text-foreground">{project.name}</p>
        <p className="text-sm text-muted-foreground">
          This project does not have any available repositories yet.
        </p>
        <UnavailableProjectRepositories project={project} />
      </div>
    );
  }

  const repoContributors = repoSnapshotQuery.data?.contributors ?? [];
  const displayedRepositorySnapshot =
    repoSource === "local"
      ? (localRepoSnapshotQuery.data?.snapshot ?? null)
      : repoSnapshotQuery.data;
  const displayedRepositoryContributors =
    displayedRepositorySnapshot?.contributors ?? repoContributors;
  const displayedRepositoryFiles = displayedRepositorySnapshot?.files ?? [];
  const selectedPullRequest =
    pullRequestsQuery.data?.find((item) => item.id === selectedPullRequestId) ??
    null;
  const selectedIssue =
    issuesQuery.data?.find((item) => item.id === selectedIssueId) ?? null;
  const displayedSnapshotCommits =
    repoSource === "local"
      ? (localRepoSnapshotQuery.data?.snapshot.commits ?? [])
      : (repoSnapshotQuery.data?.commits ?? []);
  const selectedCommit = selectedCommitHash
    ? (displayedSnapshotCommits.find(
        (commit) => commit.hash === selectedCommitHash,
      ) ?? null)
    : null;
  const { activeTabCrumb, activeWorkItemCrumb, handleGoToProjectHome } =
    buildProjectDetailCrumbs({
      activeTab,
      commit: selectedCommit,
      issue: selectedIssue,
      pullRequest: selectedPullRequest,
      setRequestedTab,
      setSelectedCommitHash,
      setSelectedIssueId,
      setSelectedPullRequestId,
      setTabsResetKey,
    });
  const agentPageContext = buildProjectDetailAgentContext({
    activeTab,
    branch: activeBranch,
    file: filesContext,
    project,
    repository,
    source: repoSource,
    workItems: [selectedCommit, selectedIssue, selectedPullRequest],
  });
  const repositoryPanelAction = (
    <ProjectRightPanelControls
      collapsed={repositoryPanel.collapsed}
      mode={repositoryPanel.mode}
      onCollapse={repositoryPanel.collapse}
      onExpand={repositoryPanel.expand}
      onModeChange={repositoryPanel.setMode}
      terminalAvailable={projectTerminalContext !== null}
    />
  );
  const detachedRepositoryPanel =
    !profilePanelPubkey &&
    !repositoryPanel.collapsed &&
    repositoryPanel.mode === "repository";
  const sharedHeaderBackdrop =
    !selectedPullRequestId && !selectedIssueId && !selectedCommitHash;
  const handleRepositoryChange = (nextRepositoryId: string) => {
    applyRepositorySearch({
      repositoryId: nextRepositoryId,
      issueId: null,
      pullRequestId: null,
      commitHash: null,
    });
    setSelectedPullRequestId(null);
    setSelectedIssueId(null);
    setSelectedCommitHash(null);
    setRequestedTab(undefined);
    setRepoSource("remote");
    setTabsResetKey((key) => key + 1);
  };

  return (
    <ProfilePanelProvider onOpenProfilePanel={handleOpenProfilePanel}>
      <ProjectBranchActionDialogs
        actions={branchActions}
        activeBranch={activeBranch}
        activeBranchCommit={activeBranchCommit}
        existingBranches={branchOptionsWithLocal}
      />
      <div className="flex min-h-0 min-w-0 flex-1 flex-row overflow-hidden">
        <ProjectConversationPanelController
          canResetWidth={threadPanelWidth.canReset}
          closeWhen={Boolean(profilePanelPubkey)}
          detachFallbackPanel={detachedRepositoryPanel}
          fallbackPanel={
            profilePanelPubkey || repositoryPanel.collapsed ? null : (
              <ProjectDetailRightPanel
                activeTab={activeTab}
                canResetWidth={activeRightPanelWidth.canReset}
                contributors={displayedRepositoryContributors}
                context={agentPageContext}
                createIssuePending={createIssueMutation.isPending}
                detachedRepository={detachedRepositoryPanel}
                files={displayedRepositoryFiles}
                identityPubkey={identityPubkey}
                issues={issuesQuery.data ?? []}
                mode={repositoryPanel.mode}
                onCreateTask={() => {
                  setCreateIssueRequestKey((key) => key + 1);
                }}
                onOpenTerminal={() => {
                  void handleOpenTerminal();
                }}
                onOpenLocalRepository={() => {
                  void handleOpenLocalRepository();
                }}
                onRepositoryChange={handleRepositoryChange}
                onResetWidth={activeRightPanelWidth.onResetWidth}
                onResizeStart={activeRightPanelWidth.onResizeStart}
                profiles={profiles}
                pullRequests={pullRequestsQuery.data ?? []}
                project={project}
                projects={projectsQuery.data ?? []}
                repository={repository}
                sharedHeaderBackdrop={sharedHeaderBackdrop}
                snapshot={displayedRepositorySnapshot}
                sourceControls={filesSourceControls}
                terminalTitle={projectTerminalLabel(hasLocalCheckout)}
                widthPx={activeRightPanelWidth.widthPx}
              />
            )
          }
          onOpenConversation={handleCloseProfilePanel}
          onResetWidth={threadPanelWidth.onResetWidth}
          onResizeStart={threadPanelWidth.onResizeStart}
          resetKey={`${repository.id}:${selectedPullRequestId ?? ""}:${selectedIssueId ?? ""}:${selectedCommitHash ?? ""}`}
          sharedHeaderBackdrop={sharedHeaderBackdrop}
          widthPx={threadPanelWidth.widthPx}
        >
          <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden [container-type:inline-size]">
            <ProjectDetailChrome
              actions={repositoryPanelAction}
              activeTabCrumb={activeTabCrumb}
              activeWorkItemCrumb={activeWorkItemCrumb}
              onGoProjectHome={handleGoToProjectHome}
              onGoProjects={() => {
                void goProjects();
              }}
              project={project}
              repository={repository}
              shareTab={
                activeWorkItemCrumb
                  ? undefined
                  : shareTabForWorkspaceTab(activeTab)
              }
            />

            <div
              className="flex min-h-0 min-w-0 flex-1 flex-col overflow-x-hidden overflow-y-auto overscroll-y-none px-4 pb-4"
              data-testid="project-detail-scroll"
            >
              <div className="w-full space-y-3">
                <WorkspaceTabs
                  key={`${project.id}:${repository.id}:${tabsResetKey}`}
                  initialTab={
                    requestedTab
                      ? workspaceTabForShareTab(requestedTab)
                      : undefined
                  }
                  initialTabRequestKey={entityNavigationId}
                  fileContentSource={fileContentSource}
                  commitDiff={commitDiffQuery.data}
                  commitDiffError={commitDiffQuery.error}
                  commitDiffLoading={commitDiffQuery.isLoading}
                  contributorActivityCounts={contributorActivityCounts}
                  contributorPubkeys={contributorPubkeys}
                  createIssueAction={{
                    onCreate: handleCreateIssue,
                    pending: createIssueMutation.isPending,
                  }}
                  createIssueRequestKey={createIssueRequestKey}
                  createPullRequestAction={{
                    onCreated: handlePullRequestCreated,
                    projects: projectsQuery.data ?? [project],
                    reposDir: activeCommunity?.reposDir,
                  }}
                  updatePullRequestAction={
                    openBranchPullRequest &&
                    repoSyncStatusQuery.data?.remoteHead &&
                    repoSyncStatusQuery.data.remoteHead !==
                      openBranchPullRequest.commit
                      ? {
                          onUpdate: () => {
                            void handleUpdatePullRequest();
                          },
                          pending: updatePullRequestMutation.isPending,
                        }
                      : undefined
                  }
                  localSnapshot={localRepoSnapshotQuery.data}
                  localSnapshotError={localRepoSnapshotQuery.error}
                  localSnapshotLoading={localRepoSnapshotQuery.isLoading}
                  onBranchChange={handleBranchChange}
                  onFilesContextChange={setFilesContext}
                  onOpenMergeRecoveryTerminal={handleOpenMergeRecoveryTerminal}
                  onSelectedCommitHashChange={handleSelectedCommitHashChange}
                  onSelectedIssueIdChange={handleSelectedIssueIdChange}
                  onSelectedPullRequestIdChange={
                    handleSelectedPullRequestIdChange
                  }
                  onSelectedTabChange={setActiveTab}
                  profiles={profiles}
                  project={repository}
                  projectId={project.id}
                  repoDiff={displayedRepoDiff}
                  repoDiffError={displayedRepoDiffError}
                  repoDiffLoading={displayedRepoDiffLoading}
                  pullRequests={pullRequestsQuery.data ?? []}
                  pullRequestsError={pullRequestsQuery.error}
                  pullRequestsLoading={pullRequestsQuery.isLoading}
                  repoContributors={repoContributors}
                  repoHost={repoRemote.host}
                  repoSource={repoSource}
                  selectedCommitHash={selectedCommitHash}
                  selectedIssueId={selectedIssueId}
                  selectedPullRequestId={selectedPullRequestId}
                  sharedHeaderBackdrop={sharedHeaderBackdrop}
                  snapshot={repoSnapshotQuery.data}
                  snapshotError={repoSnapshotQuery.error}
                  snapshotLoading={repoSnapshotQuery.isLoading}
                  sourceControls={filesSourceControls}
                  viewerGitIdentity={viewerGitIdentity}
                />
              </div>
            </div>
          </div>
          {profilePanelPubkey ? (
            <UserProfilePanel
              canResetWidth={threadPanelWidth.canReset}
              currentPubkey={identityPubkey}
              onClose={handleCloseProfilePanel}
              onOpenDm={handleOpenDm}
              onOpenProfile={handleOpenProfilePanel}
              onResetWidth={threadPanelWidth.onResetWidth}
              onResizeStart={threadPanelWidth.onResizeStart}
              onTabChange={handleProfilePanelTabChange}
              onViewChange={handleProfilePanelViewChange}
              pubkey={profilePanelPubkey}
              tab={profilePanelTab}
              transparentChrome={sharedHeaderBackdrop}
              view={profilePanelView}
              widthPx={threadPanelWidth.widthPx}
            />
          ) : null}
        </ProjectConversationPanelController>
      </div>
    </ProfilePanelProvider>
  );
}
