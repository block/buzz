import {
  CheckCircle2,
  CircleDot,
  GitBranch,
  GitPullRequest,
  type LucideIcon,
  MessageCircle,
  Users,
} from "lucide-react";
import type * as React from "react";

import type {
  ProjectIssue,
  ProjectPullRequest,
  Repository,
} from "@/features/projects/hooks";

function ContextDetailRow({
  icon: Icon,
  label,
  value,
}: {
  icon: LucideIcon;
  label: string;
  value: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <dt className="flex items-center gap-3 text-muted-foreground">
        <Icon className="h-3.5 w-3.5" />
        {label}
      </dt>
      <dd className="font-medium text-foreground">{value}</dd>
    </div>
  );
}

export function ProjectWorkItemContextDetails({
  issue,
  pullRequest,
  repository,
}: {
  issue?: ProjectIssue | null;
  pullRequest?: ProjectPullRequest | null;
  repository: Repository;
}) {
  if (issue) {
    return (
      <>
        <ContextDetailRow
          icon={CircleDot}
          label="Status"
          value={issue.status}
        />
        <ContextDetailRow
          icon={Users}
          label="Assignees"
          value={issue.assignees.length}
        />
        <ContextDetailRow
          icon={MessageCircle}
          label="Comments"
          value={issue.comments.length}
        />
      </>
    );
  }

  if (pullRequest) {
    const sourceBranch = pullRequest.branchName || "Unknown branch";
    const targetBranch =
      pullRequest.targetBranch || repository.defaultBranch || "Default branch";
    const branchLabel = `${sourceBranch} → ${targetBranch}`;
    return (
      <>
        <ContextDetailRow
          icon={GitBranch}
          label="Branch"
          value={
            <span
              className="block max-w-40 truncate"
              data-testid="project-context-branch"
              title={branchLabel}
            >
              {branchLabel}
            </span>
          }
        />
        <ContextDetailRow
          icon={GitPullRequest}
          label="Status"
          value={pullRequest.status}
        />
        <ContextDetailRow
          icon={Users}
          label="Reviewers"
          value={pullRequest.reviewers.length}
        />
        <ContextDetailRow
          icon={CheckCircle2}
          label="Approvals"
          value={pullRequest.approvals.length}
        />
      </>
    );
  }

  return null;
}
