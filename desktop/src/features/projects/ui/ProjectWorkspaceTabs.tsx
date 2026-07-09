import { TerminalSquare } from "lucide-react";
import * as React from "react";

import type {
  Project,
  ProjectLocalRepoSnapshot,
  ProjectPullRequest,
  ProjectRepoContributor,
  ProjectRepoDiff,
  ProjectRepoSnapshot,
} from "@/features/projects/hooks";
import {
  commitAuthorPubkeysFromPullRequests,
  type ViewerGitIdentity,
} from "@/features/projects/lib/projectContributorMatching";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { Button } from "@/shared/ui/button";
import { Tabs, TabsContent } from "@/shared/ui/tabs";
import { findReadmeFile } from "./ProjectReadmePanel";
import { RepositoryFilesPanel } from "./ProjectRepositoryPanel";
import type { RepoSourceHeaderControls } from "./ProjectRepositorySource";
import { ProjectCommitDetailPanel } from "./ProjectCommitDetailPanel";
import { ActivityPanel, ContributorsPanel } from "./ProjectDetailFeedPanels";
import { ProjectIssuesPanel } from "./ProjectIssuesPanel";
import { ProjectOverviewPanel } from "./ProjectOverviewPanel";
import {
  PullRequestDetailHeader,
  PullRequestMetaRail,
  PullRequestsPanel,
} from "./ProjectPullRequestsPanel";
import {
  ProjectTabsList,
  PullRequestTabsList,
} from "./ProjectWorkspaceTabList";
import { ProjectPullRequestFilesChangedPanel } from "./ProjectPullRequestFilesChangedPanel";

export function WorkspaceTabs({
  commitDiff,
  commitDiffError,
  commitDiffLoading,
  localSnapshot,
  localSnapshotError,
  localSnapshotLoading,
  project,
  repoDiff,
  repoDiffError,
  repoDiffLoading,
  selectedCommitHash,
  selectedIssueId,
  selectedPullRequestId,
  pullRequests,
  pullRequestsError,
  pullRequestsLoading,
  onSelectedCommitHashChange,
  onSelectedIssueIdChange,
  onSelectedPullRequestIdChange,
  onSelectedTabChange,
  onBranchChange,
  onOpenTerminal,
  snapshot,
  snapshotError,
  snapshotLoading,
  profiles,
  repoContributors,
  repoSource,
  sourceControls,
  terminalTitle,
  viewerGitIdentity,
}: {
  commitDiff: ProjectRepoDiff | null | undefined;
  commitDiffError: unknown;
  commitDiffLoading: boolean;
  localSnapshot: ProjectLocalRepoSnapshot | null | undefined;
  localSnapshotError: unknown;
  localSnapshotLoading: boolean;
  project: Project;
  repoDiff: ProjectRepoDiff | null | undefined;
  repoDiffError: unknown;
  repoDiffLoading: boolean;
  selectedCommitHash: string | null;
  selectedIssueId: string | null;
  selectedPullRequestId: string | null;
  pullRequests: ProjectPullRequest[];
  pullRequestsError: unknown;
  pullRequestsLoading: boolean;
  onSelectedCommitHashChange: (hash: string | null) => void;
  onSelectedIssueIdChange: (id: string | null) => void;
  onSelectedPullRequestIdChange: (id: string | null) => void;
  /** Reports the active tab so the screen breadcrumb can mirror it. */
  onSelectedTabChange?: (tab: string) => void;
  onBranchChange: (branch: string | null) => void;
  onOpenTerminal?: () => void;
  snapshot: ProjectRepoSnapshot | null | undefined;
  snapshotError: unknown;
  snapshotLoading: boolean;
  profiles?: UserProfileLookup;
  repoContributors: ProjectRepoContributor[];
  repoSource: "remote" | "local";
  /** Branch picker + remote/local toggle for the Code tab header. */
  sourceControls?: RepoSourceHeaderControls;
  terminalTitle?: string;
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
  const files = displayedSnapshot?.files ?? [];
  const readmeFile = React.useMemo(() => findReadmeFile(files), [files]);
  const commitAuthorPubkeys = React.useMemo(
    () => commitAuthorPubkeysFromPullRequests(pullRequests),
    [pullRequests],
  );
  const selectedPullRequest =
    pullRequests.find(
      (pullRequest) => pullRequest.id === selectedPullRequestId,
    ) ?? null;
  const isPullRequestSelected = Boolean(selectedPullRequest);
  const [selectedTab, setSelectedTab] = React.useState("overview");

  React.useEffect(() => {
    onSelectedTabChange?.(selectedTab);
  }, [onSelectedTabChange, selectedTab]);

  React.useEffect(() => {
    if (isPullRequestSelected) {
      setSelectedTab((currentTab) =>
        currentTab.startsWith("pr-") ? currentTab : "pr-conversation",
      );
      if (selectedPullRequest?.branchName) {
        onBranchChange(selectedPullRequest.branchName);
      }
    } else {
      setSelectedTab((currentTab) =>
        currentTab.startsWith("pr-") ? "prs" : currentTab,
      );
    }
  }, [isPullRequestSelected, onBranchChange, selectedPullRequest?.branchName]);

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
      if (!nextTab.startsWith("pr-")) {
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

  return (
    <Tabs
      className="space-y-3"
      onValueChange={handleTabChange}
      value={selectedTab}
    >
      <div className="flex min-w-0 items-center gap-1">
        <ProjectTabsList prsActive={isPullRequestSelected} />
        {onOpenTerminal ? (
          <Button
            aria-label="Open terminal"
            className="h-8 w-8 shrink-0 text-muted-foreground hover:text-foreground"
            onClick={onOpenTerminal}
            size="icon"
            title={terminalTitle ?? "Open terminal"}
            variant="ghost"
          >
            <TerminalSquare className="h-3.5 w-3.5" />
          </Button>
        ) : null}
      </div>

      {selectedPullRequest ? (
        <div className="overflow-hidden rounded-xl border border-border/60 bg-card">
          <PullRequestDetailHeader
            profiles={profiles}
            pullRequest={selectedPullRequest}
          />
          <div className="border-b border-border/60 px-4">
            <PullRequestTabsList
              filesCount={repoDiff?.files.length ?? files.length}
              pullRequest={selectedPullRequest}
            />
          </div>
          {(["conversation", "commits", "checks"] as const).map((mode) => (
            <TabsContent className="m-0" key={mode} value={`pr-${mode}`}>
              <div className="grid xl:grid-cols-[minmax(0,1fr)_18rem]">
                <div className="min-w-0">
                  <PullRequestsPanel
                    error={pullRequestsError}
                    isLoading={pullRequestsLoading}
                    mode={mode}
                    onSelectedPullRequestIdChange={
                      onSelectedPullRequestIdChange
                    }
                    profiles={profiles}
                    project={project}
                    pullRequests={pullRequests}
                    selectedPullRequestId={selectedPullRequestId}
                  />
                </div>
                <PullRequestMetaRail
                  profiles={profiles}
                  project={project}
                  pullRequest={selectedPullRequest}
                />
              </div>
            </TabsContent>
          ))}
          <TabsContent className="m-0" value="pr-files">
            <ProjectPullRequestFilesChangedPanel
              diff={repoDiff}
              error={repoDiffError}
              isLoading={repoDiffLoading}
              pullRequest={selectedPullRequest}
            />
          </TabsContent>
        </div>
      ) : null}

      <TabsContent className="m-0" value="overview">
        <ProjectOverviewPanel
          contributors={displayedContributors}
          files={files}
          onViewContributors={() => setSelectedTab("contributors")}
          profiles={profiles}
          project={project}
          pullRequests={pullRequests}
          readmeFile={readmeFile}
          snapshot={displayedSnapshot}
          sourceControls={sourceControls}
        />
      </TabsContent>

      <TabsContent className="m-0" value="activity">
        {selectedCommitHash ? (
          <ProjectCommitDetailPanel
            commit={
              displayedSnapshot?.commits.find(
                (commit) => commit.hash === selectedCommitHash,
              ) ?? null
            }
            commitAuthorPubkeys={commitAuthorPubkeys}
            commitHash={selectedCommitHash}
            viewerGitIdentity={viewerGitIdentity}
            diff={commitDiff}
            diffError={commitDiffError}
            diffLoading={commitDiffLoading}
            profiles={profiles}
          />
        ) : (
          <ActivityPanel
            error={displayedSnapshotError}
            isLoading={displayedSnapshotLoading}
            onSelectCommit={(commit) => onSelectedCommitHashChange(commit.hash)}
            profiles={profiles}
            pullRequests={pullRequests}
            repoContributors={displayedContributors}
            snapshot={displayedSnapshot}
            viewerGitIdentity={viewerGitIdentity}
          />
        )}
      </TabsContent>

      <TabsContent
        className="m-0 overflow-hidden rounded-xl border border-border/60 bg-card"
        value="prs"
      >
        <PullRequestsPanel
          error={pullRequestsError}
          isLoading={pullRequestsLoading}
          onSelectedPullRequestIdChange={onSelectedPullRequestIdChange}
          profiles={profiles}
          project={project}
          pullRequests={pullRequests}
          selectedPullRequestId={selectedPullRequestId}
        />
      </TabsContent>

      <TabsContent
        className="m-0 overflow-hidden rounded-xl border border-border/60 bg-card"
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
        {repoSource === "local" && !localSnapshot && !localSnapshotLoading ? (
          <div className="mb-3">
            <div className="border border-border/60 bg-card p-4 text-sm text-muted-foreground">
              No local checkout found.
            </div>
          </div>
        ) : null}
        <RepositoryFilesPanel
          error={displayedSnapshotError}
          fallbackAuthorPubkey={project.owner}
          files={files}
          isLoading={displayedSnapshotLoading}
          profiles={profiles}
          snapshot={displayedSnapshot}
          sourceControls={sourceControls}
        />
      </TabsContent>

      <TabsContent className="m-0" value="contributors">
        <ContributorsPanel
          profiles={profiles}
          repoContributors={displayedContributors}
        />
      </TabsContent>
    </Tabs>
  );
}
