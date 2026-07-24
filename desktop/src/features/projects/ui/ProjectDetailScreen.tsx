import { ArrowLeft, ExternalLink, FolderGit2 } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useOpenDmMutation } from "@/features/channels/hooks";
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
import { useProfileQuery, useUsersBatchQuery } from "@/features/profile/hooks";
import { mergeCurrentProfileIntoLookup } from "@/features/profile/lib/identity";
import {
  type ProfilePanelTab,
  type ProfilePanelView,
  UserProfilePanel,
} from "@/features/profile/ui/UserProfilePanel";
import {
  profilePanelTabFromSearch,
  profilePanelViewFromSearch,
} from "@/features/profile/ui/UserProfilePanelUtils";
import { useIdentityQuery } from "@/shared/api/hooks";
import { openProjectMergeRecoveryTerminal } from "@/shared/api/projectGit";
import { useMainInsetRef } from "@/shared/layout/MainInsetContext";
import { channelContentTopPaddingMeasurement } from "@/shared/layout/chromeLayout";
import { useMeasuredCssVariable } from "@/shared/layout/useMeasuredCssVariable";
import { isSafeUrl } from "@/shared/lib/url";
import { ProfilePanelProvider } from "@/shared/context/ProfilePanelContext";
import { useHistorySearchState } from "@/shared/hooks/useHistorySearchState";
import { useThreadPanelWidth } from "@/shared/hooks/useThreadPanelWidth";
import { Button } from "@/shared/ui/button";
import { useCommunities } from "@/features/communities/useCommunities";
import { useProjectCommitDiffQuery } from "@/features/projects/useProjectCommitDiff";
import { useGitIdentityQuery } from "@/features/projects/useGitIdentity";
import type { ViewerGitIdentity } from "@/features/projects/lib/projectContributorMatching";
import {
  projectBranchCreationReason,
  projectBranchManagementState,
  projectBranchOptionsFromSync,
  resolveProjectDefaultBranch,
} from "@/features/projects/lib/projectBranches";
import { normalizeRepositoryUrl } from "@/features/projects/lib/projectsViewHelpers";
import { selectProjectRepository } from "@/features/projects/projectModels";
import { WorkspaceTabs } from "./ProjectWorkspaceTabs";
import { ProjectRepositoryPicker } from "./ProjectRepositoryPicker";
import type { RepoSourceHeaderControls } from "./ProjectRepositorySource";
import {
  projectTerminalLabel,
  useOpenProjectTerminal,
} from "./useOpenProjectTerminal";
import type { CreateIssueDialogInput } from "./CreateIssueDialog";
import { ProjectBranchActionDialogs } from "./ProjectBranchActionDialogs";
import { ProjectDetailChrome } from "./ProjectDetailChrome";
import {
  PROJECT_TAB_CRUMB_LABELS,
  projectPeople,
  pushPullTitle,
  snapshotHasContent,
} from "./projectDetailHelpers";

type ProjectDetailScreenProps = {
  commitHash?: string;
  projectId: string;
  pullRequestId?: string;
  issueId?: string;
  repositoryId?: string;
};

const PROJECT_DETAIL_PANEL_SEARCH_KEYS = [
  "profile",
  "profileTab",
  "profileView",
] as const;
const PROJECT_REPOSITORY_SEARCH_KEYS = [
  "repositoryId",
  "issueId",
  "pullRequestId",
  "commitHash",
] as const;

export function ProjectDetailScreen(props: ProjectDetailScreenProps) {
  const { commitHash, projectId, pullRequestId, issueId, repositoryId } = props;
  const { goChannel, goProject, goProjects } = useAppNavigation();
  const { activeCommunity } = useCommunities();
  const mainInsetRef = useMainInsetRef();
  const projectDetailHeaderChromeRef = useMeasuredCssVariable({
    targetRef: mainInsetRef,
    resetKey: projectId,
    ...channelContentTopPaddingMeasurement,
  });
  const projectQuery = useProjectQuery(projectId);
  const projectsQuery = useProjectsQuery();
  const project = projectQuery.data;
  const repository = selectProjectRepository(project, repositoryId);
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
      tags: repoStateQuery.data?.tags ?? [],
    });
  const activeTag =
    repoStateQuery.data?.tags.find((tag) => tag.name === selectedTag) ?? null;
  const [selectedPullRequestId, setSelectedPullRequestId] = React.useState<
    string | null
  >(pullRequestId ?? null);
  React.useEffect(
    () => setSelectedPullRequestId(pullRequestId ?? null),
    [pullRequestId],
  );
  const [selectedIssueId, setSelectedIssueId] = React.useState<string | null>(
    issueId ?? null,
  );
  React.useEffect(() => setSelectedIssueId(issueId ?? null), [issueId]);
  const [selectedCommitHash, setSelectedCommitHash] = React.useState<
    string | null
  >(commitHash ?? null);
  React.useEffect(
    () => setSelectedCommitHash(commitHash ?? null),
    [commitHash],
  );
  // Bumped when breadcrumb navigation should land on the project Overview
  // tab; remounts WorkspaceTabs, which owns the selected-tab state.
  const [tabsResetKey, setTabsResetKey] = React.useState(0);
  // Mirror of the WorkspaceTabs selection so the breadcrumb can name the
  // active sub-tab. The Overview (readme) tab is "home" and gets no crumb.
  const [activeTab, setActiveTab] = React.useState("overview");
  // Commit, PR, and issue details are mutually exclusive views, so opening
  // one clears the others.
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
  const repoSnapshotQuery = useProjectRepoSnapshotQuery(
    repository,
    activeBranch,
    selectedTag ? null : selectedBranchPullRequest,
    activeTag,
  );
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
      toast.error("Could not fetch repository.", {
        description:
          error instanceof Error ? error.message : "The Git fetch failed.",
      });
      return;
    }
    toast.success("Remote state refreshed.");
  }, [repoSnapshotQuery, repoStateQuery, repoSyncStatusQuery]);
  // Compact branch + remote/local controls shared by the readme and Files
  // tab headers.
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
    remoteLabel: repoSnapshotQuery.isLoading ? "Remote checking" : "Remote",
    onCloneLocal:
      !selectedTag && repository?.cloneUrls[0]
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
  const projectPending = projectQuery.isPending;
  React.useEffect(() => {
    if (!repository) {
      // While the project query is still loading, keep the URL-seeded
      // pullRequestId/issueId selections — clearing here would discard them
      // before the detail view ever gets a chance to open.
      if (projectPending) return;
      setSelectedPullRequestId(null);
      setSelectedIssueId(null);
      setSelectedCommitHash(null);
    }
  }, [projectPending, repository]);
  React.useEffect(() => {
    setRepoSource((currentSource) => {
      if (selectedTag) return "remote";
      if (currentSource === "local" && !hasLocalCheckout) return "remote";
      if (
        currentSource === "remote" &&
        !hasRemoteSnapshot &&
        hasLocalCheckout
      ) {
        return "local";
      }
      return currentSource;
    });
  }, [hasLocalCheckout, hasRemoteSnapshot, selectedTag]);
  const peoplePubkeys = React.useMemo(() => {
    if (!repository) return [];
    // Include PR authors/updaters so commit rows can resolve avatars for
    // publishers who are not listed as project contributors.
    const pullRequestPubkeys = (pullRequestsQuery.data ?? []).flatMap(
      (pullRequest) => [
        pullRequest.author,
        ...pullRequest.updates.map((update) => update.author),
        ...pullRequest.comments.map((comment) => comment.author),
        ...pullRequest.reviewers,
        ...pullRequest.approvals.map((approval) => approval.author),
      ],
    );
    const issuePubkeys = (issuesQuery.data ?? []).flatMap((issue) => [
      issue.author,
      ...issue.recipients,
      ...issue.comments.map((comment) => comment.author),
    ]);
    return [
      ...new Set([
        ...projectPeople(repository),
        ...pullRequestPubkeys,
        ...issuePubkeys,
      ]),
    ];
  }, [issuesQuery.data, pullRequestsQuery.data, repository]);
  const profilesQuery = useUsersBatchQuery(peoplePubkeys, {
    enabled: peoplePubkeys.length > 0,
  });
  const currentProfileQuery = useProfileQuery();
  const profiles = React.useMemo(
    () =>
      mergeCurrentProfileIntoLookup(
        profilesQuery.data?.profiles,
        currentProfileQuery.data,
      ),
    [currentProfileQuery.data, profilesQuery.data?.profiles],
  );
  const identityQuery = useIdentityQuery();
  const gitIdentityQuery = useGitIdentityQuery();
  const viewerGitIdentity = React.useMemo<ViewerGitIdentity | null>(() => {
    const pubkey = identityQuery.data?.pubkey ?? null;
    if (!pubkey || !gitIdentityQuery.data) return null;
    return {
      pubkey,
      name: gitIdentityQuery.data.name,
      email: gitIdentityQuery.data.email,
    };
  }, [gitIdentityQuery.data, identityQuery.data?.pubkey]);
  const { applyPatch, values } = useHistorySearchState(
    PROJECT_DETAIL_PANEL_SEARCH_KEYS,
  );
  const profilePanelPubkey = values.profile;
  const profilePanelTab = profilePanelTabFromSearch(values.profileTab);
  const profilePanelView = profilePanelViewFromSearch(values.profileView);
  const handleOpenProfilePanel = React.useCallback(
    (pubkey: string) =>
      applyPatch({ profile: pubkey, profileTab: null, profileView: null }),
    [applyPatch],
  );
  const handleCloseProfilePanel = React.useCallback(
    () => applyPatch({ profile: null, profileTab: null, profileView: null }),
    [applyPatch],
  );
  const handleProfilePanelViewChange = React.useCallback(
    (view: ProfilePanelView, options?: { replace?: boolean }) =>
      applyPatch({ profileView: view === "summary" ? null : view }, options),
    [applyPatch],
  );
  const handleProfilePanelTabChange = React.useCallback(
    (tab: ProfilePanelTab, options?: { replace?: boolean }) =>
      applyPatch({ profileTab: tab === "info" ? null : tab }, options),
    [applyPatch],
  );
  const threadPanelWidth = useThreadPanelWidth();
  const openDmMutation = useOpenDmMutation();
  const handleOpenDm = React.useCallback(
    async (pubkeys: string[]) => {
      const dm = await openDmMutation.mutateAsync({ pubkeys });
      await goChannel(dm.id);
    },
    [goChannel, openDmMutation],
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
            ? `${result.message} Pull request updated.`
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
      toast.error(
        error instanceof Error ? error.message : "Failed to clone repository",
      );
    }
  }, [cloneRepoMutation]);
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
    async ({ body, title }: CreateIssueDialogInput) => {
      const issueId = await createIssueMutation.mutateAsync({ body, title });
      toast.success("Issue created.");
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
      toast.success(
        updated ? "Pull request updated." : "Pull request is already current.",
      );
      await pullRequestsQuery.refetch();
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to update pull request",
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
  const openTerminal = useOpenProjectTerminal(activeCommunity?.reposDir);
  const handleOpenTerminal = React.useCallback(() => {
    if (!repository) return Promise.resolve();
    return openTerminal(repository, {
      branch: activeBranch,
      hasLocalCheckout,
    });
  }, [activeBranch, hasLocalCheckout, openTerminal, repository]);
  const handleOpenMergeRecoveryTerminal = React.useCallback(
    async (input: {
      expectedCommit: string;
      sourceBranch: string;
      sourceCloneUrl: string;
      targetBranch: string;
    }) => {
      const targetCloneUrl = repository?.cloneUrls[0];
      if (!repository || !targetCloneUrl) {
        throw new Error("No project selected.");
      }
      return openProjectMergeRecoveryTerminal({
        ...input,
        projectDtag: repository.dtag,
        reposDir: activeCommunity?.reposDir,
        targetCloneUrl,
      });
    },
    [activeCommunity?.reposDir, repository],
  );

  if (projectQuery.isLoading) {
    return null;
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
      </div>
    );
  }

  const repoContributors = repoSnapshotQuery.data?.contributors ?? [];
  const safeWebUrl =
    repository.webUrl && isSafeUrl(repository.webUrl)
      ? repository.webUrl
      : null;
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

  // The active work item drives the breadcrumb trail: Projects › project ›
  // sub-tab › title. `clear` steps back to the item's list tab. Categories
  // match the workspace tab labels.
  const activeWorkItemCrumb = selectedPullRequest
    ? {
        category: "Pull Request",
        title: selectedPullRequest.title,
        clear: () => setSelectedPullRequestId(null),
      }
    : selectedIssue
      ? {
          category: "Issues",
          title: selectedIssue.title,
          clear: () => setSelectedIssueId(null),
        }
      : selectedCommitHash
        ? {
            category: "Commits",
            title: selectedCommit?.subject ?? selectedCommitHash.slice(0, 7),
            clear: () => setSelectedCommitHash(null),
          }
        : null;
  // Sub-tab crumb when no work item is open. Overview (readme) is home.
  const activeTabCrumb = activeWorkItemCrumb
    ? null
    : (PROJECT_TAB_CRUMB_LABELS[activeTab] ?? null);
  const handleGoToProjectHome = () => {
    setSelectedPullRequestId(null);
    setSelectedIssueId(null);
    setSelectedCommitHash(null);
    // Remount the workspace tabs so the project page opens on Overview
    // instead of whatever tab the work item left behind.
    setTabsResetKey((key) => key + 1);
  };
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
        <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
          <ProjectDetailChrome
            activeTabCrumb={activeTabCrumb}
            activeWorkItemCrumb={activeWorkItemCrumb}
            chromeRef={projectDetailHeaderChromeRef}
            onGoChannel={(channelId) => {
              void goChannel(channelId);
            }}
            onGoProjectHome={handleGoToProjectHome}
            onGoProjects={() => {
              void goProjects();
            }}
            project={project}
          />

          <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto px-4 pb-4">
            <div className="w-full space-y-3 pt-[calc(var(--buzz-channel-content-top-padding,5.75rem)_+_1px)]">
              <section className="space-y-3">
                <div className="flex min-w-0 items-start justify-between gap-3">
                  <div className="min-w-0 flex-1 space-y-0.5">
                    <div className="flex min-w-0 items-center gap-1.5">
                      <h2 className="truncate text-xl font-semibold tracking-tight">
                        {project.name}
                      </h2>
                      {safeWebUrl ? (
                        <Button
                          asChild
                          aria-label="Open project web page"
                          className="h-6 w-6 shrink-0 text-muted-foreground hover:text-foreground"
                          size="icon-xs"
                          variant="ghost"
                        >
                          <a
                            href={safeWebUrl}
                            rel="noopener noreferrer"
                            target="_blank"
                          >
                            <ExternalLink className="h-3.5 w-3.5" />
                          </a>
                        </Button>
                      ) : null}
                    </div>
                  </div>
                  <ProjectRepositoryPicker
                    onChange={handleRepositoryChange}
                    project={project}
                    repository={repository}
                  />
                </div>
              </section>

              <WorkspaceTabs
                key={`${project.id}:${repository.id}:${tabsResetKey}`}
                commitDiff={commitDiffQuery.data}
                commitDiffError={commitDiffQuery.error}
                commitDiffLoading={commitDiffQuery.isLoading}
                createIssueAction={{
                  onCreate: handleCreateIssue,
                  pending: createIssueMutation.isPending,
                }}
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
                onOpenMergeRecoveryTerminal={handleOpenMergeRecoveryTerminal}
                onOpenTerminal={() => {
                  void handleOpenTerminal();
                }}
                terminalTitle={projectTerminalLabel(hasLocalCheckout)}
                onSelectedCommitHashChange={handleSelectedCommitHashChange}
                onSelectedIssueIdChange={handleSelectedIssueIdChange}
                onSelectedPullRequestIdChange={
                  handleSelectedPullRequestIdChange
                }
                onSelectedTabChange={setActiveTab}
                profiles={profiles}
                project={repository}
                repoDiff={displayedRepoDiff}
                repoDiffError={displayedRepoDiffError}
                repoDiffLoading={displayedRepoDiffLoading}
                pullRequests={pullRequestsQuery.data ?? []}
                pullRequestsError={pullRequestsQuery.error}
                pullRequestsLoading={pullRequestsQuery.isLoading}
                repoContributors={repoContributors}
                repoSource={repoSource}
                selectedCommitHash={selectedCommitHash}
                selectedIssueId={selectedIssueId}
                selectedPullRequestId={selectedPullRequestId}
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
            currentPubkey={identityQuery.data?.pubkey}
            onClose={handleCloseProfilePanel}
            onOpenDm={handleOpenDm}
            onOpenProfile={handleOpenProfilePanel}
            onResetWidth={threadPanelWidth.onResetWidth}
            onResizeStart={threadPanelWidth.onResizeStart}
            onTabChange={handleProfilePanelTabChange}
            onViewChange={handleProfilePanelViewChange}
            pubkey={profilePanelPubkey}
            tab={profilePanelTab}
            view={profilePanelView}
            widthPx={threadPanelWidth.widthPx}
          />
        ) : null}
      </div>
    </ProfilePanelProvider>
  );
}
