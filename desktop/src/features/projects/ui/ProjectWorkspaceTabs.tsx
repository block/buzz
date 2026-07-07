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
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Tabs, TabsContent } from "@/shared/ui/tabs";
import { findReadmeFile, RepositoryFilesPanel } from "./ProjectRepositoryPanel";
import { ProjectCommitDetailPanel } from "./ProjectCommitDetailPanel";
import { ActivityPanel, ContributorsPanel } from "./ProjectDetailFeedPanels";
import { ProjectIssuesPanel } from "./ProjectIssuesPanel";
import { ProjectOverviewPanel } from "./ProjectOverviewPanel";
import {
  PullRequestDetailHeader,
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
  onBranchChange,
  onOpenTerminal,
  snapshot,
  snapshotError,
  snapshotLoading,
  profiles,
  repoContributors,
  repoSource,
  terminalTitle,
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
  onBranchChange: (branch: string | null) => void;
  onOpenTerminal?: () => void;
  snapshot: ProjectRepoSnapshot | null | undefined;
  snapshotError: unknown;
  snapshotLoading: boolean;
  profiles?: UserProfileLookup;
  repoContributors: ProjectRepoContributor[];
  repoSource: "remote" | "local";
  terminalTitle?: string;
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
  const selectedPullRequest =
    pullRequests.find(
      (pullRequest) => pullRequest.id === selectedPullRequestId,
    ) ?? null;
  const isPullRequestSelected = Boolean(selectedPullRequest);
  const [selectedTab, setSelectedTab] = React.useState("overview");

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
      if (!nextTab.startsWith("pr-") && nextTab !== "prs") {
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
      {selectedPullRequest ? (
        <div className="space-y-4">
          <PullRequestDetailHeader
            profiles={profiles}
            project={project}
            pullRequest={selectedPullRequest}
          />
          <PullRequestTabsList
            filesCount={repoDiff?.files.length ?? files.length}
            pullRequest={selectedPullRequest}
          />
        </div>
      ) : (
        <div className="flex min-w-0 items-center justify-between gap-2">
          <ProjectTabsList />
          {onOpenTerminal ? (
            <Button
              className="h-8 shrink-0 gap-1.5 rounded-full px-3 text-muted-foreground hover:text-foreground"
              onClick={onOpenTerminal}
              size="sm"
              title={terminalTitle}
              variant="ghost"
            >
              <TerminalSquare className="h-3.5 w-3.5" />
              Terminal
            </Button>
          ) : null}
        </div>
      )}

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
        />
      </TabsContent>

      <TabsContent
        className={cn(
          "m-0",
          !selectedCommitHash &&
            "overflow-hidden rounded-xl border border-border/50 bg-card/60",
        )}
        value="activity"
      >
        {selectedCommitHash ? (
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
            profiles={profiles}
          />
        ) : (
          <ActivityPanel
            error={displayedSnapshotError}
            isLoading={displayedSnapshotLoading}
            onSelectCommit={(commit) => onSelectedCommitHashChange(commit.hash)}
            profiles={profiles}
            repoContributors={displayedContributors}
            snapshot={displayedSnapshot}
          />
        )}
      </TabsContent>

      <TabsContent
        className="m-0 overflow-hidden rounded-xl border border-border/50 bg-card/60"
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
        className="m-0 overflow-hidden rounded-xl border border-border/50 bg-card/60"
        value="issues"
      >
        <ProjectIssuesPanel
          onSelectedIssueIdChange={onSelectedIssueIdChange}
          profiles={profiles}
          project={project}
          selectedIssueId={selectedIssueId}
        />
      </TabsContent>

      {(["conversation", "commits", "checks"] as const).map((mode) => (
        <TabsContent
          className="m-0 overflow-hidden rounded-xl border border-border/50 bg-card/60"
          key={mode}
          value={`pr-${mode}`}
        >
          <PullRequestsPanel
            error={pullRequestsError}
            isLoading={pullRequestsLoading}
            mode={mode}
            onSelectedPullRequestIdChange={onSelectedPullRequestIdChange}
            profiles={profiles}
            project={project}
            pullRequests={pullRequests}
            selectedPullRequestId={selectedPullRequestId}
          />
        </TabsContent>
      ))}

      <TabsContent className="m-0" value="files">
        {repoSource === "local" && !localSnapshot && !localSnapshotLoading ? (
          <div className="mb-3">
            <div className="rounded-xl border border-border/50 bg-card/60 p-4 text-sm text-muted-foreground">
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
        />
      </TabsContent>

      <TabsContent className="m-0" value="pr-files">
        <ProjectPullRequestFilesChangedPanel
          diff={repoDiff}
          error={repoDiffError}
          isLoading={repoDiffLoading}
          pullRequest={selectedPullRequest}
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
