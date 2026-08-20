import {
  CheckCircle2,
  CircleDot,
  GitPullRequest,
  type LucideIcon,
  MessageCircle,
  Users,
} from "lucide-react";
import type * as React from "react";

import type {
  ProjectIssue,
  ProjectPullRequest,
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
}: {
  issue?: ProjectIssue | null;
  pullRequest?: ProjectPullRequest | null;
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
    return (
      <>
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
