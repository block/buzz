import * as React from "react";

import {
  useProjectIssuesQuery,
  useProjectPullRequestsQuery,
  useProjectRepoSnapshotQuery,
  useRepoStateQuery,
  type Project,
} from "@/features/projects/hooks";
import { gitContributorPubkeysFromCommits } from "@/features/projects/lib/projectContributorMatching";
import { resolveProjectDefaultBranch } from "@/features/projects/lib/projectBranches";
import type { ProjectHomeWorkspaceSheetTab } from "@/features/projects/lib/projectHomeWorkspaceSheet";
import { ActivityPanel, ContributorsPanel } from "./ProjectDetailFeedPanels";
import { ProjectHomeCodebasePanel } from "./ProjectHomeCodebasePanel";
import { ProjectIssuesPanel } from "./ProjectIssuesPanel";
import { PullRequestsPanel } from "./ProjectPullRequestsPanel";
import { useProjectDetailPeople } from "./useProjectDetailPeople";

export function ProjectHomeWorkspaceSheet({
  identityPubkey,
  onOpenCommit,
  onRepositoryAdded,
  onSelectRepository,
  project,
  projects,
  repository,
  tab,
}: {
  identityPubkey?: string;
  onOpenCommit: (commitHash: string) => void;
  onRepositoryAdded: (repositoryId: string) => void;
  onSelectRepository: (repositoryId: string) => void;
  project: Project;
  projects: Project[];
  repository: Project["repositories"][number];
  tab: ProjectHomeWorkspaceSheetTab;
}) {
  const [selectedIssueId, setSelectedIssueId] = React.useState<string | null>(
    null,
  );
  const [selectedPullRequestId, setSelectedPullRequestId] = React.useState<
    string | null
  >(null);

  const issuesQuery = useProjectIssuesQuery(repository);
  const pullRequestsQuery = useProjectPullRequestsQuery(repository);
  const issues = issuesQuery.data ?? [];
  const pullRequests = pullRequestsQuery.data ?? [];
  const people = useProjectDetailPeople({
    issues,
    pullRequests,
    repository,
  });
  const repoStateQuery = useRepoStateQuery(repository);
  const defaultBranch = resolveProjectDefaultBranch(
    repository.defaultBranch,
    repoStateQuery.data,
  );
  const snapshotQuery = useProjectRepoSnapshotQuery(
    repository,
    defaultBranch,
    null,
    null,
    true,
  );
  const snapshot = snapshotQuery.data ?? null;
  const contributorPubkeysByGitIdentity = React.useMemo(
    () =>
      gitContributorPubkeysFromCommits(snapshot?.commits ?? [], pullRequests),
    [pullRequests, snapshot?.commits],
  );
  const selectedPullRequest =
    pullRequests.find(
      (pullRequest) => pullRequest.id === selectedPullRequestId,
    ) ?? null;

  let body: React.ReactNode;
  switch (tab) {
    case "issues":
      body = (
        <ProjectIssuesPanel
          onSelectedIssueIdChange={setSelectedIssueId}
          profiles={people.profiles}
          project={repository}
          selectedIssueId={selectedIssueId}
        />
      );
      break;
    case "prs":
      body = (
        <PullRequestsPanel
          error={pullRequestsQuery.error}
          isLoading={pullRequestsQuery.isLoading}
          onSelectedPullRequestIdChange={setSelectedPullRequestId}
          profiles={people.profiles}
          project={repository}
          pullRequests={pullRequests}
          selectedPullRequest={selectedPullRequest}
        />
      );
      break;
    case "commits":
      body = (
        <ActivityPanel
          branch={defaultBranch ?? undefined}
          error={snapshotQuery.error}
          isLoading={snapshotQuery.isPending}
          onSelectCommit={(commit) => onOpenCommit(commit.hash)}
          profiles={people.profiles}
          project={repository}
          projectId={project.id}
          pullRequests={pullRequests}
          repoContributors={snapshot?.contributors ?? []}
          snapshot={snapshot}
          viewerGitIdentity={people.viewerGitIdentity}
        />
      );
      break;
    case "files":
      body = (
        <ProjectHomeCodebasePanel
          identityPubkey={identityPubkey}
          onOpenCommit={onOpenCommit}
          onRepositoryAdded={onRepositoryAdded}
          onSelectRepository={onSelectRepository}
          project={project}
          projects={projects}
          repository={repository}
        />
      );
      break;
    case "contributors":
      body = (
        <ContributorsPanel
          activityCounts={people.contributorActivityCounts}
          contributorPubkeys={people.contributorPubkeys}
          contributorPubkeysByGitIdentity={contributorPubkeysByGitIdentity}
          profiles={people.profiles}
          repoContributors={snapshot?.contributors ?? []}
        />
      );
      break;
  }

  return (
    <div data-tab={tab} data-testid="project-home-workspace-sheet">
      {body}
    </div>
  );
}
