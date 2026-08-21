import { useIsManagedAgent } from "@/features/agent-memory/hooks";
import type {
  ProjectIssue,
  ProjectPullRequest,
  Repository,
} from "@/features/projects/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { useIdentityQuery } from "@/shared/api/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { IssueAssigneesRow } from "./IssueAssigneesRow";
import { PullRequestReviewersRow } from "./PullRequestReviewersRow";

export function ProjectWorkItemContextActions({
  issue,
  profiles,
  pullRequest,
  repository,
}: {
  issue?: ProjectIssue | null;
  profiles?: UserProfileLookup;
  pullRequest?: ProjectPullRequest | null;
  repository: Repository;
}) {
  const identityQuery = useIdentityQuery();
  const viewerPubkey = identityQuery.data?.pubkey;
  const viewer = viewerPubkey ? normalizePubkey(viewerPubkey) : null;
  const isOwner = viewer === normalizePubkey(repository.owner);
  const isManagedAgentOwner = useIsManagedAgent(repository.owner) === true;

  if (issue) {
    if (!viewer) return null;
    const isAuthor = viewer === normalizePubkey(issue.author);
    const canAssignOthers =
      viewer !== null && (isAuthor || isOwner || isManagedAgentOwner);
    return (
      <section
        className="space-y-1 pt-2"
        data-testid="project-context-task-assignment"
      >
        <h3 className="text-xs font-normal text-muted-foreground/70">
          Assignment
        </h3>
        <IssueAssigneesRow
          canAssignOthers={canAssignOthers}
          contextActions
          issue={issue}
          profiles={profiles}
          project={repository}
          signAsManagedOwner={isManagedAgentOwner && !isOwner}
          showAssignees={false}
          showSelfAssignmentState
          testIdPrefix="project-context-issue"
          viewerPubkey={viewer}
        />
      </section>
    );
  }

  if (pullRequest) {
    const isAuthor = viewer === normalizePubkey(pullRequest.author);
    const canRequestReview =
      viewer !== null &&
      (pullRequest.status === "Open" || pullRequest.status === "Draft") &&
      (isAuthor || isOwner || isManagedAgentOwner);
    if (!canRequestReview) return null;
    return (
      <section className="pt-2" data-testid="project-context-reviewers">
        <PullRequestReviewersRow
          actionLabel="Add Reviewer"
          canRequest={canRequestReview}
          contextActions
          profiles={profiles}
          project={repository}
          pullRequest={pullRequest}
          signAsManagedOwner={isManagedAgentOwner && !isOwner}
          showDecisionActors={false}
          showSummary={false}
          summaryTestId="project-context-review-summary"
        />
      </section>
    );
  }

  return null;
}
