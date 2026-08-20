import { GitBranch, GitMerge, GitPullRequest, Tag } from "lucide-react";

import { useIsManagedAgent } from "@/features/agent-memory/hooks";
import type {
  ProjectPullRequest,
  Repository as Project,
} from "@/features/projects/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { useIdentityQuery } from "@/shared/api/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { CopyCommitHashButton } from "./ProjectCommitCopyButton";
import {
  ProjectDetailMetaList,
  ProjectDetailMetaPills,
  ProjectDetailMetaRow,
} from "./ProjectDetailMeta";
import { PullRequestReviewersRow } from "./PullRequestReviewersRow";

function statusClassName(status: ProjectPullRequest["status"]) {
  if (status === "Closed") return "text-destructive";
  if (status === "Draft") return "text-muted-foreground";
  if (status === "Merged") return "text-purple-400";
  return "text-green-500";
}

/** Compact metadata under the review title: branches, reviewers, status. */
export function PullRequestMetaHeader({
  diffStats,
  profiles,
  project,
  pullRequest,
}: {
  diffStats?: { additions: number; deletions: number } | null;
  profiles?: UserProfileLookup;
  project: Project;
  pullRequest: ProjectPullRequest;
}) {
  const identityQuery = useIdentityQuery();
  const targetBranch =
    pullRequest.targetBranch || project.defaultBranch || "default branch";
  const sourceBranch = pullRequest.branchName || "unknown branch";
  const labels = pullRequest.labels.filter(
    (label) => label.toLowerCase() !== "draft",
  );
  const viewerPubkey = identityQuery.data?.pubkey;
  const viewer = viewerPubkey ? normalizePubkey(viewerPubkey) : null;
  const isAuthor = viewer === normalizePubkey(pullRequest.author);
  const isOwner = viewer === normalizePubkey(project.owner);
  const isManagedAgentOwner = useIsManagedAgent(project.owner) === true;
  const canRequestReview =
    Boolean(viewer) && (isAuthor || isOwner || isManagedAgentOwner);
  const StatusIcon =
    pullRequest.status === "Merged" ? GitMerge : GitPullRequest;

  return (
    <ProjectDetailMetaList>
      <ProjectDetailMetaRow icon={GitBranch} label="Branch">
        <p className="flex min-w-0 flex-wrap items-center gap-1.5">
          <span className="truncate text-foreground">{sourceBranch}</span>
          <span aria-hidden className="text-muted-foreground">
            →
          </span>
          <span className="truncate text-foreground">{targetBranch}</span>
          {diffStats ? (
            <>
              <span className="text-green-500">+{diffStats.additions}</span>
              <span className="text-destructive">-{diffStats.deletions}</span>
            </>
          ) : null}
          {pullRequest.commit ? (
            <span className="inline-flex min-w-0 items-center gap-0.5 text-muted-foreground">
              <code
                className="truncate font-mono text-xs"
                title={pullRequest.commit}
              >
                {pullRequest.commit.slice(0, 7)}
              </code>
              <CopyCommitHashButton
                className="h-5 w-5 p-0"
                hash={pullRequest.commit}
              />
            </span>
          ) : null}
        </p>
      </ProjectDetailMetaRow>
      <PullRequestReviewersRow
        canRequest={canRequestReview}
        profiles={profiles}
        project={project}
        pullRequest={pullRequest}
        signAsManagedOwner={isManagedAgentOwner && !isOwner}
      />
      <ProjectDetailMetaRow icon={StatusIcon} label="Status">
        <span className={`font-medium ${statusClassName(pullRequest.status)}`}>
          {pullRequest.status}
        </span>
      </ProjectDetailMetaRow>
      {labels.length > 0 ? (
        <ProjectDetailMetaRow icon={Tag} label="Labels">
          <ProjectDetailMetaPills labels={labels} />
        </ProjectDetailMetaRow>
      ) : null}
    </ProjectDetailMetaList>
  );
}
