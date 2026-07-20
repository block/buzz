import {
  ArrowLeft,
  ChevronRight,
  ExternalLink,
  FolderGit2,
  MessageSquare,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useOpenDmMutation } from "@/features/channels/hooks";
import {
  type Project,
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
import {
  channelChrome,
  channelContentTopPaddingMeasurement,
  topChromeInset,
} from "@/shared/layout/chromeLayout";
import { useMeasuredCssVariable } from "@/shared/layout/useMeasuredCssVariable";
import { cn } from "@/shared/lib/cn";
import { isSafeUrl } from "@/shared/lib/url";
import { ProfilePanelProvider } from "@/shared/context/ProfilePanelContext";
import { useHistorySearchState } from "@/shared/hooks/useHistorySearchState";
import { useThreadPanelWidth } from "@/shared/hooks/useThreadPanelWidth";
import { Button } from "@/shared/ui/button";
import { useCommunities } from "@/features/communities/useCommunities";
import { useProjectCommitDiffQuery } from "@/features/projects/useProjectCommitDiff";
import { useGitIdentityQuery } from "@/features/projects/useGitIdentity";
import type { ViewerGitIdentity } from "@/features/projects/lib/projectContributorMatching";
import { normalizeRepositoryUrl } from "@/features/projects/lib/projectsViewHelpers";
import { WorkspaceTabs } from "./ProjectWorkspaceTabs";
import type { RepoSourceHeaderControls } from "./ProjectRepositorySource";
import {
  projectTerminalLabel,
  useOpenProjectTerminal,
} from "./useOpenProjectTerminal";
import type { CreateIssueDialogInput } from "./CreateIssueDialog";
import {
  projectPeople,
  pushPullTitle,
  snapshotHasContent,
} from "./projectDetailHelpers";

type ProjectDetailScreenProps = {
  commitHash?: string;
  projectId: string;
  pullRequestId?: string;
  issueId?: string;
};

const PROJECT_DETAIL_PANEL_SEARCH_KEYS = [
  "profile",
  "profileTab",
  "profileView",
] as const;

export function ProjectDetailScreen(props: ProjectDetailScreenProps) {
  const { commitHash, projectId, pullRequestId, issueId } = props;
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
  const repoStateQuery = useRepoStateQuery(project);
  const pullRequestsQuery = useProjectPullRequestsQuery(project);
  const branchOptions = React.useMemo(() => {
    const names = [
      project?.defaultBranch,
      ...(repoStateQuery.data?.branches.map((branch) => branch.name) ?? []),
      ...(pullRequestsQuery.data
        ?.map((pullRequest) => pullRequest.branchName)
        .filter((name): name is string => Boolean(name)) ?? []),
    ].filter((name): name is string => Boolean(name));
    return [...new Set(names)];
  }, [
    project?.defaultBranch,
    pullRequestsQuery.data,
    repoStateQuery.data?.branches,
  ]);
  const [selectedBranch, setSelectedBranch] = React.useState<string | null>(
    null,
  );
  const activeBranch =
    selectedBranch ?? project?.defaultBranch ?? branchOptions[0] ?? null;
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
  const issuesQuery = useProjectIssuesQuery(project);
  const selectedBranchPullRequest = React.useMemo(() => {
    const projectRepositories = new Set(
      (project?.cloneUrls ?? []).map(normalizeRepositoryUrl),
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
  }, [activeBranch, project?.cloneUrls, pullRequestsQuery.data]);
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
    project,
    activeBranch,
    selectedBranchPullRequest,
  );
  const repoDiffQuery = useProjectRepoDiffQuery(
    project,
    activeBranch,
    activeRepoPullRequest,
    repoSource === "remote",
  );
  const localRepoDiffQuery = useProjectLocalRepoDiffQuery(
    project,
    activeCommunity?.reposDir,
    activeBranch,
    activeRepoPullRequest,
    repoSource === "local" && Boolean(activeRepoPullRequest),
  );
  const commitDiffQuery = useProjectCommitDiffQuery(
    project,
    selectedCommitHash,
    repoSource,
    activeCommunity?.reposDir,
  );
  const localRepoSnapshotQuery = useProjectLocalRepoSnapshotQuery(
    project,
    activeCommunity?.reposDir,
    activeBranch,
  );
  const repoSyncStatusQuery = useProjectRepoSyncStatusQuery(
    project,
    activeCommunity?.reposDir,
    activeBranch,
  );
  const pushLocalRepoMutation = usePushProjectLocalRepositoryMutation(
    project,
    activeCommunity?.reposDir,
    activeBranch,
    openBranchPullRequest,
  );
  const pullLocalRepoMutation = usePullProjectLocalRepositoryMutation(
    project,
    activeCommunity?.reposDir,
    activeBranch,
  );
  const cloneRepoMutation = useCloneProjectRepositoryMutation(
    project,
    activeCommunity?.reposDir,
  );
  const createIssueMutation = useCreateProjectIssueMutation(project);
  const updatePullRequestMutation = useUpdateProjectPullRequestMutation(
    project,
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
  const branchOptionsWithLocal = React.useMemo(
    () => [
      ...new Set(
        [...branchOptions, repoSyncStatusQuery.data?.localBranch].filter(
          (branch): branch is string => Boolean(branch),
        ),
      ),
    ],
    [branchOptions, repoSyncStatusQuery.data?.localBranch],
  );
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
    onBranchChange: setSelectedBranch,
    source: repoSource,
    onSourceChange: setRepoSource,
    localDisabled:
      !repoSyncStatusQuery.data?.localPath &&
      !localRepoSnapshotQuery.data &&
      !localRepoSnapshotQuery.isLoading,
    localLabel: localRepoSnapshotQuery.isLoading
      ? "Local checking"
      : repoSyncStatusQuery.data?.localPath || localRepoSnapshotQuery.data
        ? "Local"
        : "Local missing",
    remoteLabel: repoSnapshotQuery.isLoading ? "Remote checking" : "Remote",
    onCloneLocal: project?.cloneUrls[0]
      ? () => {
          void handleCloneRepo();
        }
      : undefined,
    clonePending: cloneRepoMutation.isPending,
    canPush: repoSyncStatusQuery.data?.canPush ?? false,
    onPush: () => {
      void handlePushLocalRepo();
    },
    pushDisabled:
      pushLocalRepoMutation.isPending || !repoSyncStatusQuery.data?.canPush,
    pushPending: pushLocalRepoMutation.isPending,
    pushTitle:
      repoSyncStatusQuery.data?.pushBlockReason ??
      pushPullTitle("Push", repoSyncStatusQuery.data?.aheadCount, "local"),
    canPull: repoSyncStatusQuery.data?.canPull ?? false,
    onPull: () => {
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
    if (!project) {
      // While the project query is still loading, keep the URL-seeded
      // pullRequestId/issueId selections — clearing here would discard them
      // before the detail view ever gets a chance to open.
      if (projectPending) return;
      setSelectedBranch(null);
      setSelectedPullRequestId(null);
      setSelectedIssueId(null);
      setSelectedCommitHash(null);
      return;
    }
    setSelectedBranch((currentBranch) => {
      if (currentBranch && branchOptions.includes(currentBranch)) {
        return currentBranch;
      }
      return project.defaultBranch ?? branchOptions[0] ?? null;
    });
  }, [project, branchOptions, projectPending]);
  React.useEffect(() => {
    setRepoSource((currentSource) => {
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
  }, [hasLocalCheckout, hasRemoteSnapshot]);
  const peoplePubkeys = React.useMemo(() => {
    if (!project) return [];
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
        ...projectPeople(project),
        ...pullRequestPubkeys,
        ...issuePubkeys,
      ]),
    ];
  }, [issuesQuery.data, project, pullRequestsQuery.data]);
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
    async (createdProject: Project, pullRequestId: string) => {
      if (createdProject.id !== projectId) {
        await goProject(createdProject.id, { pullRequestId });
        return;
      }
      await pullRequestsQuery.refetch();
      setSelectedPullRequestId(pullRequestId);
    },
    [goProject, projectId, pullRequestsQuery],
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
    if (!project) return Promise.resolve();
    return openTerminal(project, {
      branch: activeBranch,
      hasLocalCheckout,
    });
  }, [activeBranch, hasLocalCheckout, openTerminal, project]);
  const handleOpenMergeRecoveryTerminal = React.useCallback(
    async (input: {
      expectedCommit: string;
      sourceBranch: string;
      sourceCloneUrl: string;
      targetBranch: string;
    }) => {
      const targetCloneUrl = project?.cloneUrls[0];
      if (!project || !targetCloneUrl) {
        throw new Error("No project selected.");
      }
      return openProjectMergeRecoveryTerminal({
        ...input,
        projectDtag: project.dtag,
        reposDir: activeCommunity?.reposDir,
        targetCloneUrl,
      });
    },
    [activeCommunity?.reposDir, project],
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

  const repoContributors = repoSnapshotQuery.data?.contributors ?? [];
  const safeWebUrl =
    project.webUrl && isSafeUrl(project.webUrl) ? project.webUrl : null;
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
  const TAB_CRUMB_LABELS: Record<string, string> = {
    files: "Files",
    activity: "Commits",
    issues: "Issues",
    prs: "Pull Request",
    contributors: "Contributors",
  };
  const activeTabCrumb = activeWorkItemCrumb
    ? null
    : (TAB_CRUMB_LABELS[activeTab] ?? null);
  const handleGoToProjectHome = () => {
    setSelectedPullRequestId(null);
    setSelectedIssueId(null);
    setSelectedCommitHash(null);
    // Remount the workspace tabs so the project page opens on Overview
    // instead of whatever tab the work item left behind.
    setTabsResetKey((key) => key + 1);
  };

  return (
    <ProfilePanelProvider onOpenProfilePanel={handleOpenProfilePanel}>
      <div className="flex min-h-0 min-w-0 flex-1 flex-row overflow-hidden">
        <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
          <div
            className={cn(
              "pointer-events-none relative z-30 overflow-hidden rounded-tl-xl bg-background/80 backdrop-blur-md supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55",
              channelChrome.negativeMargin,
              topChromeInset.divider,
            )}
            ref={projectDetailHeaderChromeRef}
          >
            <div
              className="pointer-events-auto flex min-h-[2.75rem] items-center justify-between gap-3 px-4 py-1.5"
              data-tauri-drag-region
            >
              <nav
                aria-label="Project breadcrumb"
                className="-ml-1 flex min-w-0 items-center gap-0.5 text-xs text-muted-foreground"
              >
                <button
                  className="flex shrink-0 items-center gap-1.5 rounded-md px-1 py-1 font-medium transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                  onClick={() => {
                    void goProjects();
                  }}
                  type="button"
                >
                  <FolderGit2 className="h-3.5 w-3.5" />
                  Projects
                </button>
                <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground/60" />
                {activeWorkItemCrumb ? (
                  <>
                    <button
                      className="min-w-0 truncate rounded-md px-0.5 py-1 font-medium transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                      onClick={handleGoToProjectHome}
                      type="button"
                    >
                      {project.name}
                    </button>
                    <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground/60" />
                    <button
                      className="shrink-0 rounded-md px-0.5 py-1 font-medium transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                      onClick={activeWorkItemCrumb.clear}
                      type="button"
                    >
                      {activeWorkItemCrumb.category}
                    </button>
                    <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground/60" />
                    <span
                      aria-current="page"
                      className="min-w-0 truncate px-0.5 font-medium text-muted-foreground/60"
                    >
                      {activeWorkItemCrumb.title}
                    </span>
                  </>
                ) : activeTabCrumb ? (
                  <>
                    <button
                      className="min-w-0 truncate rounded-md px-0.5 py-1 font-medium transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                      onClick={handleGoToProjectHome}
                      type="button"
                    >
                      {project.name}
                    </button>
                    <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground/60" />
                    <span
                      aria-current="page"
                      className="min-w-0 truncate px-0.5 font-medium text-muted-foreground/60"
                    >
                      {activeTabCrumb}
                    </span>
                  </>
                ) : (
                  <span
                    aria-current="page"
                    className="min-w-0 truncate px-0.5 font-medium text-muted-foreground/60"
                  >
                    {project.name}
                  </span>
                )}
              </nav>
              {project.projectChannelId ? (
                <Button
                  className="h-8 shrink-0 gap-1.5"
                  onClick={() => {
                    if (project.projectChannelId) {
                      void goChannel(project.projectChannelId);
                    }
                  }}
                  size="sm"
                  variant="outline"
                >
                  <MessageSquare className="h-4 w-4" />
                  Open Discussion
                </Button>
              ) : null}
            </div>
          </div>

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
                </div>
              </section>

              <WorkspaceTabs
                key={`${project.id}:${tabsResetKey}`}
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
                onBranchChange={setSelectedBranch}
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
                project={project}
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
