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
  type ProjectRepoSnapshot,
  useProjectQuery,
  useProjectIssuesQuery,
  useProjectLocalRepoDiffQuery,
  useProjectLocalRepoSnapshotQuery,
  useProjectRepoDiffQuery,
  useProjectRepoSyncStatusQuery,
  usePushProjectLocalRepositoryMutation,
  useProjectPullRequestsQuery,
  useProjectRepoSnapshotQuery,
  useRepoStateQuery,
} from "@/features/projects/hooks";
import { useProfileQuery, useUsersBatchQuery } from "@/features/profile/hooks";
import {
  mergeCurrentProfileIntoLookup,
  resolveUserLabel,
} from "@/features/profile/lib/identity";
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
import { useMainInsetRef } from "@/shared/layout/MainInsetContext";
import {
  channelChrome,
  channelContentTopPaddingMeasurement,
  topChromeInset,
} from "@/shared/layout/chromeLayout";
import { useMeasuredCssVariable } from "@/shared/layout/useMeasuredCssVariable";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { isSafeUrl } from "@/shared/lib/url";
import { ProfilePanelProvider } from "@/shared/context/ProfilePanelContext";
import { useHistorySearchState } from "@/shared/hooks/useHistorySearchState";
import { useThreadPanelWidth } from "@/shared/hooks/useThreadPanelWidth";
import { Button } from "@/shared/ui/button";
import { useWorkspaces } from "@/features/workspaces/useWorkspaces";
import { useProjectCommitDiffQuery } from "@/features/projects/useProjectCommitDiff";
import { WorkspaceTabs } from "./ProjectWorkspaceTabs";
import { RepositorySourceCard } from "./ProjectRepositorySource";
import {
  projectTerminalLabel,
  useOpenProjectTerminal,
} from "./useOpenProjectTerminal";
import { ProfileIdentityButton } from "./ProjectProfileIdentity";

function projectPeople(project: Project) {
  return [
    ...new Set(
      [project.owner, ...project.contributors]
        .filter(Boolean)
        .map(normalizePubkey),
    ),
  ];
}

function snapshotHasContent(snapshot: ProjectRepoSnapshot | null | undefined) {
  return Boolean(
    snapshot &&
      (snapshot.latestCommit ||
        snapshot.commits.length > 0 ||
        snapshot.files.length > 0 ||
        snapshot.contributors.length > 0),
  );
}

type ProjectDetailScreenProps = {
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
  const { projectId, pullRequestId, issueId } = props;
  const { goChannel, goProjects } = useAppNavigation();
  const { activeWorkspace } = useWorkspaces();
  const mainInsetRef = useMainInsetRef();
  const projectDetailHeaderChromeRef = useMeasuredCssVariable({
    targetRef: mainInsetRef,
    resetKey: projectId,
    ...channelContentTopPaddingMeasurement,
  });
  const projectQuery = useProjectQuery(projectId);
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
  >(null);
  // Bumped when breadcrumb navigation should land on the project Overview
  // tab; remounts WorkspaceTabs, which owns the selected-tab state.
  const [tabsResetKey, setTabsResetKey] = React.useState(0);
  // Commit selection has no URL param, so reset it when navigating to a
  // different project within the same mounted route.
  const commitProjectIdRef = React.useRef(projectId);
  React.useEffect(() => {
    if (commitProjectIdRef.current !== projectId) {
      commitProjectIdRef.current = projectId;
      setSelectedCommitHash(null);
    }
  }, [projectId]);
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
  const selectedBranchPullRequest = React.useMemo(
    () =>
      pullRequestsQuery.data?.find(
        (pullRequest) => pullRequest.branchName === activeBranch,
      ) ?? null,
    [activeBranch, pullRequestsQuery.data],
  );
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
    activeWorkspace?.reposDir,
    activeBranch,
    activeRepoPullRequest,
    repoSource === "local" && Boolean(activeRepoPullRequest),
  );
  const commitDiffQuery = useProjectCommitDiffQuery(
    project,
    selectedCommitHash,
    repoSource,
    activeWorkspace?.reposDir,
  );
  const localRepoSnapshotQuery = useProjectLocalRepoSnapshotQuery(
    project,
    activeWorkspace?.reposDir,
    activeBranch,
  );
  const repoSyncStatusQuery = useProjectRepoSyncStatusQuery(
    project,
    activeWorkspace?.reposDir,
    activeBranch,
  );
  const pushLocalRepoMutation = usePushProjectLocalRepositoryMutation(
    project,
    activeWorkspace?.reposDir,
    activeBranch,
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
  const isWorkItemDetailOpen = Boolean(
    selectedPullRequestId || selectedIssueId || selectedCommitHash,
  );
  React.useEffect(() => {
    if (!project) {
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
  }, [project, branchOptions]);
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
  const peoplePubkeys = React.useMemo(
    () => (project ? projectPeople(project) : []),
    [project],
  );
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
      toast.success(result.message);
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

  const openTerminal = useOpenProjectTerminal(activeWorkspace?.reposDir);
  const handleOpenTerminal = React.useCallback(() => {
    if (!project) return Promise.resolve();
    return openTerminal(project, {
      branch: activeBranch,
      hasLocalCheckout,
    });
  }, [activeBranch, hasLocalCheckout, openTerminal, project]);

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

  const ownerProfile = profiles?.[normalizePubkey(project.owner)];
  const ownerLabel = resolveUserLabel({ pubkey: project.owner, profiles });
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
  // category › title. `clear` steps back to the item's list tab.
  const activeWorkItemCrumb = selectedPullRequest
    ? {
        category: "Pull request",
        title: selectedPullRequest.title,
        clear: () => setSelectedPullRequestId(null),
      }
    : selectedIssue
      ? {
          category: "Issue",
          title: selectedIssue.title,
          clear: () => setSelectedIssueId(null),
        }
      : selectedCommitHash
        ? {
            category: "Commit",
            title: selectedCommit?.subject ?? selectedCommitHash.slice(0, 7),
            clear: () => setSelectedCommitHash(null),
          }
        : null;
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
              className="pointer-events-auto flex min-h-[3.25rem] items-center justify-between gap-3 px-4 py-2"
              data-tauri-drag-region
            >
              <nav
                aria-label="Project breadcrumb"
                className="-ml-1 flex min-w-0 items-center gap-0.5 text-xs text-muted-foreground"
              >
                <button
                  aria-label={
                    activeWorkItemCrumb
                      ? `Back to ${project.name}`
                      : "Back to projects"
                  }
                  className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                  onClick={() => {
                    // One step back: work item detail → project page,
                    // project page → all projects.
                    if (activeWorkItemCrumb) {
                      activeWorkItemCrumb.clear();
                    } else {
                      void goProjects();
                    }
                  }}
                  type="button"
                >
                  <ArrowLeft className="h-3.5 w-3.5" />
                </button>
                <button
                  className="shrink-0 rounded-md px-0.5 py-1 font-medium transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                  onClick={() => {
                    void goProjects();
                  }}
                  type="button"
                >
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
                    <span className="min-w-0 truncate px-0.5 font-medium text-foreground">
                      {activeWorkItemCrumb.title}
                    </span>
                  </>
                ) : (
                  <span className="min-w-0 truncate px-0.5 font-medium text-foreground">
                    {project.name}
                  </span>
                )}
              </nav>
              {project.projectChannelId ? (
                <Button
                  className="h-9 shrink-0 gap-1.5"
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
            <div className="w-full space-y-5 pt-[calc(var(--buzz-channel-content-top-padding,5.75rem)_+_1px)]">
              {/* When a PR or issue is open, the breadcrumb already names the
                  project — let the work item's own title lead the page and
                  skip the project header section entirely. */}
              {!isWorkItemDetailOpen ? (
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
                      {project.description ? (
                        <p className="max-w-2xl text-sm font-normal text-muted-foreground">
                          {project.description}
                        </p>
                      ) : null}
                    </div>
                  </div>
                  <RepositorySourceCard
                    branch={activeBranch ?? ""}
                    branchOptions={branchOptions}
                    cloneUrls={project.cloneUrls}
                    localDisabled={
                      !repoSyncStatusQuery.data?.localPath &&
                      !localRepoSnapshotQuery.data &&
                      !localRepoSnapshotQuery.isLoading
                    }
                    localLabel={
                      localRepoSnapshotQuery.isLoading
                        ? "Local checking"
                        : repoSyncStatusQuery.data?.localPath ||
                            localRepoSnapshotQuery.data
                          ? "Local"
                          : "Local missing"
                    }
                    localPath={
                      repoSyncStatusQuery.data?.localPath ??
                      localRepoSnapshotQuery.data?.path
                    }
                    onBranchChange={setSelectedBranch}
                    onPush={() => {
                      void handlePushLocalRepo();
                    }}
                    onSourceChange={setRepoSource}
                    pushDisabled={
                      pushLocalRepoMutation.isPending ||
                      !repoSyncStatusQuery.data?.canPush
                    }
                    pushPending={pushLocalRepoMutation.isPending}
                    remoteLabel={
                      repoSnapshotQuery.isLoading ? "Remote checking" : "Remote"
                    }
                    source={repoSource}
                    status={repoSyncStatusQuery.data}
                  />
                </section>
              ) : null}

              <WorkspaceTabs
                key={`${project.id}:${tabsResetKey}`}
                commitDiff={commitDiffQuery.data}
                commitDiffError={commitDiffQuery.error}
                commitDiffLoading={commitDiffQuery.isLoading}
                localSnapshot={localRepoSnapshotQuery.data}
                localSnapshotError={localRepoSnapshotQuery.error}
                localSnapshotLoading={localRepoSnapshotQuery.isLoading}
                onBranchChange={setSelectedBranch}
                onOpenTerminal={() => {
                  void handleOpenTerminal();
                }}
                terminalTitle={projectTerminalLabel(hasLocalCheckout)}
                onSelectedCommitHashChange={handleSelectedCommitHashChange}
                onSelectedIssueIdChange={handleSelectedIssueIdChange}
                onSelectedPullRequestIdChange={
                  handleSelectedPullRequestIdChange
                }
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
              />

              <section className="flex min-w-0 items-center gap-2 rounded-xl border border-border/50 bg-card/60 px-4 py-3 text-sm text-muted-foreground">
                <span className="shrink-0 font-medium text-foreground">
                  Details:
                </span>
                <span className="min-w-0 truncate">
                  Repo: {project.repoAddress}
                </span>
                <span className="shrink-0 text-border">•</span>
                <span className="shrink-0">Creator:</span>
                <ProfileIdentityButton
                  align="center"
                  avatarClassName="mt-0.5"
                  avatarSize="xs"
                  avatarUrl={ownerProfile?.avatarUrl ?? null}
                  isAgent={ownerProfile?.isAgent === true}
                  label={ownerLabel}
                  pubkey={project.owner}
                />
              </section>
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
