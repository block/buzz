import { CircleCheck, CircleDot, CircleX, MessageSquare } from "lucide-react";

import type {
  Project,
  ProjectIssue,
  ProjectIssueListItem,
} from "@/features/projects/hooks";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";

type ProjectsIssuesListProps = {
  isLoading: boolean;
  onOpen: (project: Project, issue: ProjectIssue) => void;
  profiles?: UserProfileLookup;
  issues: ProjectIssueListItem[];
  viewMode: "grid" | "list";
};

function formatRelativeTime(createdAt: number) {
  const elapsedSeconds = Math.max(
    1,
    Math.floor(Date.now() / 1_000 - createdAt),
  );
  const units = [
    { label: "year", seconds: 365 * 24 * 60 * 60 },
    { label: "month", seconds: 30 * 24 * 60 * 60 },
    { label: "day", seconds: 24 * 60 * 60 },
    { label: "hour", seconds: 60 * 60 },
    { label: "minute", seconds: 60 },
  ];
  const unit =
    units.find((item) => elapsedSeconds >= item.seconds) ??
    units[units.length - 1];
  const value = Math.max(1, Math.floor(elapsedSeconds / unit.seconds));
  return `${value} ${unit.label}${value === 1 ? "" : "s"} ago`;
}

function issueStatusVisual(status: ProjectIssue["status"]) {
  if (status === "Done") {
    return { className: "text-purple-400", icon: CircleCheck };
  }
  if (status === "Closed") {
    return { className: "text-destructive", icon: CircleX };
  }
  return { className: "text-green-500", icon: CircleDot };
}

function nextStepLabel(status: ProjectIssue["status"]) {
  if (status === "Done" || status === "Closed") return "View issue";
  if (status === "In Review") return "Review issue";
  if (status === "Triage") return "Triage issue";
  return "Open issue";
}

function IssueHeader({
  issue,
  profiles,
  project,
}: {
  issue: ProjectIssue;
  profiles?: UserProfileLookup;
  project: Project;
}) {
  const authorLabel = resolveUserLabel({ profiles, pubkey: issue.author });
  const status = issueStatusVisual(issue.status);

  return (
    <div className="min-w-0 flex-1 space-y-1">
      <div className="flex min-w-0 items-center gap-1.5">
        <p className="truncate text-sm font-semibold text-foreground">
          {issue.title}
        </p>
        <status.icon className={`h-3.5 w-3.5 shrink-0 ${status.className}`} />
      </div>
      <p className="truncate text-xs text-muted-foreground">
        {project.name} · created {formatRelativeTime(issue.createdAt)} by{" "}
        <UserProfilePopover pubkey={issue.author} triggerElement="span">
          <button
            className="relative z-10 rounded-sm hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
            type="button"
          >
            {authorLabel}
          </button>
        </UserProfilePopover>
      </p>
    </div>
  );
}

function IssueGridCard({
  issue,
  onOpen,
  profiles,
  project,
}: {
  issue: ProjectIssue;
  onOpen: (project: Project, issue: ProjectIssue) => void;
  profiles?: UserProfileLookup;
  project: Project;
}) {
  return (
    <Card className="group relative flex min-h-40 flex-col overflow-hidden border-border/60 bg-card p-4 shadow-none transition-colors duration-150 hover:bg-muted/20">
      <button
        className="absolute inset-0"
        onClick={() => onOpen(project, issue)}
        type="button"
      >
        <span className="sr-only">View {issue.title}</span>
      </button>
      <div className="flex min-h-0 flex-1 flex-col gap-3">
        <div className="flex min-w-0 items-start gap-3">
          <IssueHeader issue={issue} profiles={profiles} project={project} />
          <Button
            className="relative z-10 h-7 shrink-0 px-2.5"
            onClick={(event) => {
              event.stopPropagation();
              onOpen(project, issue);
            }}
            size="xs"
            type="button"
            variant="outline"
          >
            {nextStepLabel(issue.status)}
          </Button>
        </div>

        {issue.content ? (
          <p className="line-clamp-2 text-sm text-foreground/90">
            {issue.content}
          </p>
        ) : null}

        <div className="mt-auto border border-border/60 bg-muted/30 px-2.5 py-2">
          <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs text-foreground/80">
            <span className="font-mono text-foreground">
              #{issue.id.slice(0, 8)}
            </span>
            {issue.comments.length > 0 ? (
              <span className="flex items-center gap-1">
                <MessageSquare className="h-3.5 w-3.5" />
                {issue.comments.length}
              </span>
            ) : null}
          </div>
        </div>
      </div>
    </Card>
  );
}

function IssueListRow({
  issue,
  onOpen,
  profiles,
  project,
}: {
  issue: ProjectIssue;
  onOpen: (project: Project, issue: ProjectIssue) => void;
  profiles?: UserProfileLookup;
  project: Project;
}) {
  return (
    <div className="group relative px-4 py-2.5 transition-colors duration-150 hover:bg-muted/20">
      <button
        className="absolute inset-0"
        onClick={() => onOpen(project, issue)}
        type="button"
      >
        <span className="sr-only">View {issue.title}</span>
      </button>
      <div className="flex min-w-0 items-start gap-3">
        <IssueHeader issue={issue} profiles={profiles} project={project} />
        <div className="relative z-10 flex shrink-0 items-center gap-2">
          <Button
            className="h-7 px-2.5"
            onClick={(event) => {
              event.stopPropagation();
              onOpen(project, issue);
            }}
            size="xs"
            type="button"
            variant="outline"
          >
            {nextStepLabel(issue.status)}
          </Button>
        </div>
      </div>
    </div>
  );
}

export function ProjectsIssuesList({
  isLoading,
  issues,
  onOpen,
  profiles,
  viewMode,
}: ProjectsIssuesListProps) {
  if (isLoading) {
    return (
      <div className="border border-border/60 px-4 py-12 text-center text-sm text-muted-foreground">
        Loading issues...
      </div>
    );
  }

  if (issues.length === 0) {
    return (
      <div className="border border-dashed border-border/60 px-4 py-12 text-center text-sm text-muted-foreground">
        No issues yet.
      </div>
    );
  }

  if (viewMode === "grid") {
    return (
      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
        {issues.map(({ project, issue }) => (
          <IssueGridCard
            issue={issue}
            key={issue.id}
            onOpen={onOpen}
            profiles={profiles}
            project={project}
          />
        ))}
      </div>
    );
  }

  return (
    <div className="-mx-4 divide-y divide-border/60 border-y border-border/60 bg-card">
      {issues.map(({ project, issue }) => (
        <IssueListRow
          issue={issue}
          key={issue.id}
          onOpen={onOpen}
          profiles={profiles}
          project={project}
        />
      ))}
    </div>
  );
}
