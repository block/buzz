import {
  CircleDot,
  Files as FilesIcon,
  GitCommitHorizontal,
  GitPullRequest,
  Hash,
  RefreshCw,
  Users,
} from "lucide-react";
import * as React from "react";

import type {
  Project,
  ProjectLocalRepoSnapshot,
  ProjectPullRequest,
  ProjectPullRequestCommentAnchor,
  ProjectRepoContributor,
  ProjectRepoDiff,
  ProjectRepoSnapshot,
  Repository,
} from "@/features/projects/hooks";
import {
  gitContributorPubkeysFromCommits,
  type ProjectContributorActivityCounts,
  type ViewerGitIdentity,
} from "@/features/projects/lib/projectContributorMatching";
import { repositoryDiscussionQuery } from "@/features/projects/lib/discussionChannels";
import type { ProjectRepoHost } from "@/features/projects/lib/projectRepoHost";
import { projectReviewFilesChangedBody } from "@/features/projects/lib/projectReviewDisplay";
import { projectRepoUnavailableReason } from "@/features/projects/lib/projectRepoAvailability";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { BuzzLoadingState } from "@/shared/ui/BuzzLoadingState";
import { Tabs, TabsContent } from "@/shared/ui/tabs";
import { findReadmeFile } from "./ProjectReadmePanel";
import { RepositoryFilesPanel } from "./ProjectRepositoryPanel";
import type { RepositoryFileContentSource } from "./useRepositoryFileContent";
import type { RepoSourceHeaderControls } from "./ProjectRepositorySource";
import { DiscussionChannelsPanel } from "./DiscussionChannels";
import { ProjectCommitDetailPanel } from "./ProjectCommitDetailPanel";
import { ActivityPanel, ContributorsPanel } from "./ProjectDetailFeedPanels";
import { ProjectIssuesPanel } from "./ProjectIssuesPanel";
import type { OpenMergeRecoveryTerminal } from "./MergePullRequestButton";
import {
  type GitDataState,
  ProjectOverviewPanel,
} from "./ProjectOverviewPanel";
import { PullRequestsPanel } from "./ProjectPullRequestsPanel";
import { ProjectTabsList } from "./ProjectWorkspaceTabList";
import { ProjectPullRequestFilesChangedPanel } from "./ProjectPullRequestFilesChangedPanel";
import { ProjectRepositoryUnavailableState } from "./ProjectRepositoryUnavailableState";
import {
  PROJECT_COLUMN_HEADER_BACKDROP_CLASS,
  PROJECT_DETAIL_PANEL_CLASS,
  PROJECT_SECTION_HEADER_CLASS,
} from "./projectPanelStyles";
import { ProjectSectionHeader } from "./ProjectSectionHeader";
import { ProjectPanelState } from "./ProjectPanelState";
import { CreatePullRequestDialog } from "./CreatePullRequestDialog";
import {
  CreateIssueDialog,
  type CreateIssueDialogInput,
} from "./CreateIssueDialog";

type CreatePullRequestAction = {
  projects: Project[];
  reposDir?: string | null;
  onCreated: (
    project: Project,
    repository: Repository,
    pullRequestId: string,
  ) => void | Promise<void>;
};

type CreateIssueAction = {
  onCreate: (input: CreateIssueDialogInput) => Promise<void>;
  pending: boolean;
};

type UpdatePullRequestAction = {
  onUpdate: () => void;
  pending: boolean;
};

export function WorkspaceTabs({
  commitDiff,
  commitDiffError,
  commitDiffLoading,
  contributorActivityCounts,
  contributorPubkeys,
  createIssueAction,
  createIssueRequestKey,
  createPullRequestAction,
  createPullRequestRequestKey,
  updatePullRequestAction,
  initialTab,
  initialFilePath,
  initialTabRequestKey,
  fileContentSource,
  localSnapshot,
  localSnapshotError,
  localSnapshotLoading,
  project,
  projectId,
  repoDiff,
  repoDiffError,
  repoDiffLoading,
  selectedCommitHash,
  selectedIssueId,
  selectedPullRequest,
  selectedPullRequestId,
  sharedHeaderBackdrop,
  pullRequests,
  pullRequestsError,
  pullRequestsLoading,
  onSelectedCommitHashChange,
  onFilesContextChange,
  onSelectedIssueIdChange,
  onSelectedPullRequestIdChange,
  onSelectedTabChange,
  onBack,
  onOpenMergeRecoveryTerminal,
  snapshot,
  snapshotError,
  snapshotLoading,
  profiles,
  repoContributors,
  repoSource,
  repoHost,
  sourceControls,
  viewerGitIdentity,
}: {
  commitDiff: ProjectRepoDiff | null | undefined;
  commitDiffError: unknown;
  commitDiffLoading: boolean;
  contributorActivityCounts: Record<string, ProjectContributorActivityCounts>;
  contributorPubkeys: string[];
  createIssueAction: CreateIssueAction;
  createIssueRequestKey?: number;
  createPullRequestAction?: CreatePullRequestAction;
  createPullRequestRequestKey?: number;
  updatePullRequestAction?: UpdatePullRequestAction;
  /** Tab to open on mount (workspace vocabulary), e.g. from a share link. */
  initialTab?: string;
  /** File or folder to open when entering the repository Files tab. */
  initialFilePath?: string;
  /** Changes for every entity-link activation, including repeated links. */
  initialTabRequestKey?: string;
  fileContentSource?: RepositoryFileContentSource;
  localSnapshot: ProjectLocalRepoSnapshot | null | undefined;
  localSnapshotError: unknown;
  localSnapshotLoading: boolean;
  project: Repository;
  projectId: string;
  repoDiff: ProjectRepoDiff | null | undefined;
  repoDiffError: unknown;
  repoDiffLoading: boolean;
  selectedCommitHash: string | null;
  selectedIssueId: string | null;
  selectedPullRequest: ProjectPullRequest | null;
  selectedPullRequestId: string | null;
  sharedHeaderBackdrop?: boolean;
  pullRequests: ProjectPullRequest[];
  pullRequestsError: unknown;
  pullRequestsLoading: boolean;
  onSelectedCommitHashChange: (hash: string | null) => void;
  onFilesContextChange?: (context: {
    kind: "file" | "folder";
    path: string;
  }) => void;
  onSelectedIssueIdChange: (id: string | null) => void;
  onSelectedPullRequestIdChange: (id: string | null) => void;
  /** Reports the active tab so the screen breadcrumb can mirror it. */
  onSelectedTabChange?: (tab: string) => void;
  onBack: () => void;
  onOpenMergeRecoveryTerminal?: OpenMergeRecoveryTerminal;
  snapshot: ProjectRepoSnapshot | null | undefined;
  snapshotError: unknown;
  snapshotLoading: boolean;
  profiles?: UserProfileLookup;
  repoContributors: ProjectRepoContributor[];
  repoSource: "remote" | "local";
  repoHost: ProjectRepoHost;
  /** Branch and source state used by repository content and recovery actions. */
  sourceControls?: RepoSourceHeaderControls;
  viewerGitIdentity?: ViewerGitIdentity | null;
}) {
  const localCheckoutSnapshot = localSnapshot?.snapshot ?? null;
  const displayedSnapshot =
    repoSource === "local" ? localCheckoutSnapshot : snapshot;
  const displayedSnapshotError =
    repoSource === "local" ? localSnapshotError : snapshotError;
  const displayedSnapshotLoading =
    repoSource === "local" ? localSnapshotLoading : snapshotLoading;
  const displayedContributors =
    displayedSnapshot?.contributors ?? repoContributors;
  const contributorPubkeysByGitIdentity = React.useMemo(
    () =>
      gitContributorPubkeysFromCommits(
        displayedSnapshot?.commits ?? [],
        pullRequests,
      ),
    [displayedSnapshot?.commits, pullRequests],
  );
  const files = displayedSnapshot?.files ?? [];
  const readmeFile = React.useMemo(() => findReadmeFile(files), [files]);
  const ownerProfile = profiles?.[normalizePubkey(project.owner)];
  const ownerName =
    ownerProfile?.displayName?.trim() ||
    ownerProfile?.nip05Handle?.trim() ||
    undefined;
  const externalHost =
    repoSource === "remote" && repoHost.kind === "external"
      ? repoHost.host
      : undefined;
  const gitDataState: GitDataState = displayedSnapshot
    ? files.length === 0
      ? "empty"
      : "available"
    : displayedSnapshotLoading
      ? "checking"
      : "unavailable";
  const unavailableReason =
    gitDataState === "unavailable" && !externalHost
      ? repoSource === "remote"
        ? (sourceControls?.remoteUnavailableReason ??
          projectRepoUnavailableReason(displayedSnapshotError))
        : displayedSnapshotError
          ? projectRepoUnavailableReason(displayedSnapshotError)
          : undefined
      : undefined;
  const repositoryUnavailableState =
    unavailableReason && !externalHost ? (
      <ProjectRepositoryUnavailableState
        accessChannelId={project.channelId}
        onAskForAccess={sourceControls?.onAskForAccess}
        onRetry={sourceControls?.onFetch}
        ownerAvatarUrl={ownerProfile?.avatarUrl}
        ownerIsAgent={ownerProfile?.isAgent}
        ownerName={ownerName}
        reason={unavailableReason}
        retryPending={sourceControls?.fetchPending}
      />
    ) : null;
  const filesChangedBody = projectReviewFilesChangedBody({
    hasPopulatedDiff: (repoDiff?.files.length ?? 0) > 0,
    hasSelectedPullRequest: Boolean(selectedPullRequest),
    repositoryUnavailable: Boolean(repositoryUnavailableState),
  });
  const selectedCommitPullRequest = React.useMemo(
    () =>
      pullRequests.find(
        (pullRequest) =>
          pullRequest.commit === selectedCommitHash ||
          pullRequest.initialCommit === selectedCommitHash,
      ),
    [pullRequests, selectedCommitHash],
  );
  const isPullRequestSelected = Boolean(selectedPullRequest);
  const isDetailSelected = Boolean(
    selectedPullRequestId || selectedIssueId || selectedCommitHash,
  );
  const [selectedTab, setSelectedTab] = React.useState(
    initialTab ?? "overview",
  );
  // Follow later share-link navigations to the same project (the search
  // param changes without a remount).
  // biome-ignore lint/correctness/useExhaustiveDependencies: request key intentionally retriggers an unchanged tab.
  React.useEffect(() => {
    if (initialTab) setSelectedTab(initialTab);
  }, [initialTab, initialTabRequestKey]);
  const [pullRequestCommentTarget, setPullRequestCommentTarget] =
    React.useState<{
      anchor: ProjectPullRequestCommentAnchor;
      pullRequestId: string;
    } | null>(null);
  const [createIssueOpen, setCreateIssueOpen] = React.useState(false);
  const previousCreateIssueRequestKey = React.useRef(createIssueRequestKey);
  const [createPullRequestOpen, setCreatePullRequestOpen] =
    React.useState(false);
  const previousCreatePullRequestRequestKey = React.useRef(
    createPullRequestRequestKey,
  );

  React.useEffect(() => {
    if (previousCreateIssueRequestKey.current === createIssueRequestKey) return;
    previousCreateIssueRequestKey.current = createIssueRequestKey;
    setCreateIssueOpen(true);
  }, [createIssueRequestKey]);

  React.useEffect(() => {
    if (
      previousCreatePullRequestRequestKey.current ===
      createPullRequestRequestKey
    ) {
      return;
    }
    previousCreatePullRequestRequestKey.current = createPullRequestRequestKey;
    setCreatePullRequestOpen(true);
  }, [createPullRequestRequestKey]);

  React.useEffect(() => {
    onSelectedTabChange?.(selectedTab);
  }, [onSelectedTabChange, selectedTab]);

  React.useEffect(() => {
    if (isPullRequestSelected) {
      setSelectedTab("prs");
    }
  }, [isPullRequestSelected]);

  React.useEffect(() => {
    if (selectedIssueId) {
      setSelectedTab("issues");
    }
  }, [selectedIssueId]);

  React.useEffect(() => {
    if (selectedCommitHash) {
      setSelectedTab("activity");
    }
  }, [selectedCommitHash]);

  const handleTabChange = React.useCallback(
    (nextTab: string) => {
      setSelectedTab(nextTab);
      if (nextTab !== "prs") {
        onSelectedPullRequestIdChange(null);
      }
      if (nextTab !== "issues") {
        onSelectedIssueIdChange(null);
      }
      if (nextTab !== "activity") {
        onSelectedCommitHashChange(null);
      }
    },
    [
      onSelectedCommitHashChange,
      onSelectedIssueIdChange,
      onSelectedPullRequestIdChange,
    ],
  );
  const handleOpenPullRequestComment = React.useCallback(
    (anchor: ProjectPullRequestCommentAnchor) => {
      if (!selectedPullRequestId) return;
      setPullRequestCommentTarget({
        anchor: { ...anchor },
        pullRequestId: selectedPullRequestId,
      });
    },
    [selectedPullRequestId],
  );
  const sectionHeader =
    selectedTab === "files" && files.length > 0 ? (
      <ProjectSectionHeader
        className={PROJECT_SECTION_HEADER_CLASS}
        icon={FilesIcon}
        title="Files"
      />
    ) : selectedTab === "activity" && !selectedCommitHash ? (
      <ProjectSectionHeader
        className={PROJECT_SECTION_HEADER_CLASS}
        icon={GitCommitHorizontal}
        title="Commits"
      />
    ) : selectedTab === "issues" && !selectedIssueId ? (
      <ProjectSectionHeader
        action={{
          disabled: createIssueAction.pending,
          label: "Create task",
          onClick: () => setCreateIssueOpen(true),
        }}
        className={PROJECT_SECTION_HEADER_CLASS}
        icon={CircleDot}
        title="Tasks"
      />
    ) : selectedTab === "prs" && !selectedPullRequestId ? (
      <ProjectSectionHeader
        action={{
          disabled:
            !createPullRequestAction ||
            createPullRequestAction.projects.length === 0,
          label: "Create review",
          onClick: () => setCreatePullRequestOpen(true),
          title: "Create review — choose a repository and branches to compare",
        }}
        className={PROJECT_SECTION_HEADER_CLASS}
        icon={GitPullRequest}
        title="Reviews"
      />
    ) : selectedTab === "channels" ? (
      <ProjectSectionHeader
        className={PROJECT_SECTION_HEADER_CLASS}
        icon={Hash}
        title="Channels"
      />
    ) : selectedTab === "contributors" ? (
      <ProjectSectionHeader
        className={PROJECT_SECTION_HEADER_CLASS}
        icon={Users}
        title="Contributors"
      />
    ) : null;

  return (
    <Tabs
      className="flex min-w-0 flex-1 flex-col space-y-3"
      onValueChange={handleTabChange}
      value={selectedTab}
    >
      {!isDetailSelected ? (
        <div
          className={`sticky top-0 z-30 -mx-4 flex h-13 min-w-0 items-center gap-1 px-4 ${
            sharedHeaderBackdrop ? "" : PROJECT_COLUMN_HEADER_BACKDROP_CLASS
          }`}
          data-testid="project-workspace-tab-menu"
        >
          <ProjectTabsList onBack={onBack} prsActive={isPullRequestSelected} />
          <div className="ml-auto flex shrink-0 items-center gap-1">
            {updatePullRequestAction ? (
              <Button
                className="h-8 shrink-0 gap-1.5"
                disabled={updatePullRequestAction.pending}
                onClick={updatePullRequestAction.onUpdate}
                size="sm"
                title="Publish the pushed commit to this review"
                variant="outline"
              >
                <RefreshCw className="h-4 w-4" />
                {updatePullRequestAction.pending
                  ? "Updating…"
                  : "Update review"}
              </Button>
            ) : null}
          </div>
        </div>
      ) : null}
      {/* Project content follows the same borderless flow as work-item details.
          Inner panels retain standalone chrome, neutralized here. */}
      <div
        className="-mx-4 flex flex-1 flex-col [&_[data-project-detail-panel]]:rounded-none [&_[data-project-detail-panel]]:border-0"
        data-testid="project-workspace-panel"
      >
        {sectionHeader}

        <TabsContent
          className="m-0 min-h-0 flex-1 flex-col data-[state=active]:flex"
          value="overview"
        >
          <ProjectOverviewPanel
            accessChannelId={project.channelId}
            externalHost={externalHost}
            externalUrl={externalHost ? sourceControls?.externalUrl : null}
            fileContentSource={fileContentSource}
            gitDataState={gitDataState}
            hideReadmeHeader
            ownerAvatarUrl={ownerProfile?.avatarUrl}
            ownerIsAgent={ownerProfile?.isAgent}
            ownerName={ownerName}
            readmeFile={readmeFile}
            sourceControls={sourceControls}
            unavailableReason={unavailableReason}
          />
        </TabsContent>

        <TabsContent
          className="m-0 min-h-0 flex-1 flex-col data-[state=active]:flex"
          value="activity"
        >
          {repositoryUnavailableState ??
            (selectedCommitHash ? (
              <ProjectCommitDetailPanel
                commit={
                  displayedSnapshot?.commits.find(
                    (commit) => commit.hash === selectedCommitHash,
                  ) ?? null
                }
                commitHash={selectedCommitHash}
                diff={commitDiff}
                diffError={commitDiffError}
                diffLoading={commitDiffLoading}
                originAgentName={selectedCommitPullRequest?.originAgentName}
                originChannelId={selectedCommitPullRequest?.channelId}
                project={project}
              />
            ) : (
              <ActivityPanel
                branch={sourceControls?.branch}
                error={displayedSnapshotError}
                isLoading={displayedSnapshotLoading}
                onSelectCommit={(commit) =>
                  onSelectedCommitHashChange(commit.hash)
                }
                profiles={profiles}
                project={project}
                projectId={projectId}
                pullRequests={pullRequests}
                repoContributors={displayedContributors}
                snapshot={displayedSnapshot}
                viewerGitIdentity={viewerGitIdentity}
              />
            ))}
        </TabsContent>

        <TabsContent
          className={`m-0 ${
            selectedPullRequestId ? "" : PROJECT_DETAIL_PANEL_CLASS
          }`}
          data-project-detail-panel
          value="prs"
        >
          <PullRequestsPanel
            diffStats={
              repoDiff
                ? {
                    additions: repoDiff.additions,
                    deletions: repoDiff.deletions,
                  }
                : null
            }
            error={pullRequestsError}
            filesChanged={
              filesChangedBody === "files" && selectedPullRequest ? (
                <ProjectPullRequestFilesChangedPanel
                  diff={repoDiff}
                  error={repoDiffError}
                  focusedAnchor={
                    pullRequestCommentTarget?.pullRequestId ===
                    selectedPullRequestId
                      ? pullRequestCommentTarget.anchor
                      : null
                  }
                  isLoading={repoDiffLoading}
                  profiles={profiles}
                  project={project}
                  pullRequest={selectedPullRequest}
                />
              ) : filesChangedBody === "unavailable" ? (
                repositoryUnavailableState
              ) : undefined
            }
            filesCount={repoDiff?.files.length}
            forceOpenFiles={
              pullRequestCommentTarget?.pullRequestId === selectedPullRequestId
            }
            isLoading={pullRequestsLoading}
            onOpenCommit={onSelectedCommitHashChange}
            onOpenInlineComment={handleOpenPullRequestComment}
            onOpenTerminal={onOpenMergeRecoveryTerminal}
            onSelectedPullRequestIdChange={onSelectedPullRequestIdChange}
            profiles={profiles}
            project={project}
            pullRequests={pullRequests}
            selectedPullRequest={selectedPullRequest}
          />
        </TabsContent>

        <TabsContent
          className={`m-0 ${selectedIssueId ? "" : PROJECT_DETAIL_PANEL_CLASS}`}
          data-project-detail-panel
          value="issues"
        >
          <ProjectIssuesPanel
            onSelectedIssueIdChange={onSelectedIssueIdChange}
            profiles={profiles}
            project={project}
            selectedIssueId={selectedIssueId}
          />
        </TabsContent>

        <TabsContent className="m-0" value="files">
          {repositoryUnavailableState ??
            (repoSource === "local" &&
            !localSnapshot &&
            !localSnapshotLoading ? (
              <ProjectPanelState
                description="Switch to the remote source or clone this repository locally."
                title="No local checkout found"
              />
            ) : (
              <RepositoryFilesPanel
                error={displayedSnapshotError}
                fallbackAuthorPubkey={project.owner}
                fileContentSource={fileContentSource}
                files={files}
                initialPath={initialFilePath}
                isLoading={displayedSnapshotLoading}
                onContextChange={onFilesContextChange}
                onOpenCommit={onSelectedCommitHashChange}
                profiles={profiles}
                snapshot={displayedSnapshot}
                unavailableMessage={
                  externalHost
                    ? `Not mirrored on Buzz. Repository files are hosted on ${externalHost}.`
                    : undefined
                }
              />
            ))}
        </TabsContent>

        <TabsContent className="m-0" value="channels">
          <DiscussionChannelsPanel
            query={repositoryDiscussionQuery(project)}
            repositoryName={project.name}
          />
        </TabsContent>

        <TabsContent className="m-0" value="contributors">
          {displayedSnapshotLoading ? (
            <BuzzLoadingState label="Loading contributors" />
          ) : (
            <ContributorsPanel
              activityCounts={contributorActivityCounts}
              contributorPubkeys={contributorPubkeys}
              contributorPubkeysByGitIdentity={contributorPubkeysByGitIdentity}
              profiles={profiles}
              repoContributors={displayedContributors}
            />
          )}
        </TabsContent>
      </div>
      {createPullRequestAction && createPullRequestOpen ? (
        <CreatePullRequestDialog
          initialProjectId={projectId}
          onCreated={createPullRequestAction.onCreated}
          onOpenChange={setCreatePullRequestOpen}
          open
          projects={createPullRequestAction.projects}
          reposDir={createPullRequestAction.reposDir}
        />
      ) : null}
      <CreateIssueDialog
        isCreating={createIssueAction.pending}
        onCreate={createIssueAction.onCreate}
        onOpenChange={setCreateIssueOpen}
        open={createIssueOpen}
        projectName={project.name}
      />
    </Tabs>
  );
}
