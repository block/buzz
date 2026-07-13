import {
  Check,
  CircleDot,
  GitCommitHorizontal,
  GitPullRequest,
  MessageSquare,
  UserPlus,
} from "lucide-react";
import type { ComponentType } from "react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type {
  Project,
  ProjectIssue,
  ProjectIssueListItem,
  ProjectPullRequest,
  ProjectPullRequestListItem,
  ProjectRepoSnapshot,
} from "@/features/projects/hooks";
import {
  formatExactTimestamp,
  relativeTime,
} from "@/features/projects/lib/projectsViewHelpers";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type ActivityKind =
  | "commit"
  | "pull-request"
  | "issue"
  | "comment"
  | "approval"
  | "review-request";

type ActivityTarget =
  | { type: "project"; project: Project }
  | {
      type: "pull-request";
      project: Project;
      pullRequest: ProjectPullRequest;
    }
  | { type: "issue"; project: Project; issue: ProjectIssue };

type ProjectActivityItem = {
  id: string;
  kind: ActivityKind;
  createdAt: number;
  actorPubkey: string | null;
  actorName: string | null;
  action: string;
  title: string;
  body: string;
  detail: string | null;
  target: ActivityTarget;
};

type ProjectsActivityFeedProps = {
  compact?: boolean;
  isLoading: boolean;
  issues: ProjectIssueListItem[];
  onOpenIssue: (project: Project, issue: ProjectIssue) => void;
  onOpenProject: (project: Project) => void;
  onOpenPullRequest: (
    project: Project,
    pullRequest: ProjectPullRequest,
  ) => void;
  profiles?: UserProfileLookup;
  projects: Project[];
  pullRequests: ProjectPullRequestListItem[];
  snapshots?: Record<string, ProjectRepoSnapshot>;
};

const ACTIVITY_LIMIT = 30;

const KIND_VISUALS: Record<
  ActivityKind,
  {
    icon: ComponentType<{ className?: string }>;
    iconClassName: string;
    badgeClassName: string;
  }
> = {
  commit: {
    icon: GitCommitHorizontal,
    iconClassName: "text-primary",
    badgeClassName: "bg-primary/10 text-primary",
  },
  "pull-request": {
    icon: GitPullRequest,
    iconClassName: "text-green-600 dark:text-green-500",
    badgeClassName:
      "bg-green-600/10 text-green-700 dark:bg-green-500/10 dark:text-green-400",
  },
  issue: {
    icon: CircleDot,
    iconClassName: "text-orange-500",
    badgeClassName: "bg-orange-500/10 text-orange-700 dark:text-orange-300",
  },
  comment: {
    icon: MessageSquare,
    iconClassName: "text-muted-foreground",
    badgeClassName: "bg-muted text-muted-foreground",
  },
  approval: {
    icon: Check,
    iconClassName: "text-green-600 dark:text-green-500",
    badgeClassName:
      "bg-green-600/10 text-green-700 dark:bg-green-500/10 dark:text-green-400",
  },
  "review-request": {
    icon: UserPlus,
    iconClassName: "text-blue-600 dark:text-blue-400",
    badgeClassName:
      "bg-blue-600/10 text-blue-700 dark:bg-blue-500/10 dark:text-blue-300",
  },
};

function contentPreview(content: string) {
  return content.replace(/\s+/g, " ").trim().slice(0, 280);
}

function buildActivityItems({
  issues,
  projects,
  pullRequests,
  snapshots,
}: Pick<
  ProjectsActivityFeedProps,
  "issues" | "projects" | "pullRequests" | "snapshots"
>) {
  const items: ProjectActivityItem[] = [];

  for (const project of projects) {
    const snapshot = snapshots?.[project.id];
    const commit = snapshot?.commits.reduce(
      (latest, candidate) =>
        !latest || candidate.timestamp > latest.timestamp ? candidate : latest,
      snapshot.commits[0],
    );
    if (!commit) continue;
    items.push({
      id: `commit:${project.id}:${commit.hash}`,
      kind: "commit",
      createdAt: commit.timestamp,
      actorPubkey: null,
      actorName: commit.authorName,
      action: `pushed a commit to ${project.name}`,
      title: commit.subject || commit.shortHash,
      body: "",
      detail: commit.shortHash,
      target: { type: "project", project },
    });
  }

  for (const { project, pullRequest } of pullRequests) {
    const target = { type: "pull-request", project, pullRequest } as const;
    items.push({
      id: `pr:${pullRequest.id}`,
      kind: "pull-request",
      createdAt: pullRequest.createdAt,
      actorPubkey: pullRequest.author,
      actorName: null,
      action: `opened a pull request in ${project.name}`,
      title: pullRequest.title,
      body: contentPreview(pullRequest.content),
      detail: pullRequest.status,
      target,
    });
    for (const update of pullRequest.updates) {
      items.push({
        id: `pr-update:${update.id}`,
        kind: "commit",
        createdAt: update.createdAt,
        actorPubkey: update.author,
        actorName: null,
        action: `updated a pull request in ${project.name}`,
        title: pullRequest.title,
        body: contentPreview(update.content),
        detail: update.commit?.slice(0, 7) ?? null,
        target,
      });
    }
    for (const comment of pullRequest.comments) {
      const kind = comment.isApproval
        ? "approval"
        : comment.isReviewRequest
          ? "review-request"
          : "comment";
      items.push({
        id: `pr-comment:${comment.id}`,
        kind,
        createdAt: comment.createdAt,
        actorPubkey: comment.author,
        actorName: null,
        action: comment.isApproval
          ? `approved a pull request in ${project.name}`
          : comment.isReviewRequest
            ? `requested review in ${project.name}`
            : `commented on a pull request in ${project.name}`,
        title: pullRequest.title,
        body: contentPreview(comment.content),
        detail: null,
        target,
      });
    }
  }

  for (const { project, issue } of issues) {
    const target = { type: "issue", project, issue } as const;
    items.push({
      id: `issue:${issue.id}`,
      kind: "issue",
      createdAt: issue.createdAt,
      actorPubkey: issue.author,
      actorName: null,
      action: `created an issue in ${project.name}`,
      title: issue.title,
      body: contentPreview(issue.content),
      detail: issue.status,
      target,
    });
    for (const comment of issue.comments) {
      items.push({
        id: `issue-comment:${comment.id}`,
        kind: "comment",
        createdAt: comment.createdAt,
        actorPubkey: comment.author,
        actorName: null,
        action: `commented on an issue in ${project.name}`,
        title: issue.title,
        body: contentPreview(comment.content),
        detail: null,
        target,
      });
    }
  }

  return items
    .sort((left, right) => right.createdAt - left.createdAt)
    .slice(0, ACTIVITY_LIMIT);
}

function ActivityCard({
  compact,
  item,
  onOpen,
  profiles,
}: {
  compact: boolean;
  item: ProjectActivityItem;
  onOpen: () => void;
  profiles?: UserProfileLookup;
}) {
  const visual = KIND_VISUALS[item.kind];
  const Icon = visual.icon;
  const profile = item.actorPubkey
    ? profiles?.[normalizePubkey(item.actorPubkey)]
    : undefined;
  const actorLabel = item.actorPubkey
    ? resolveUserLabel({ profiles, pubkey: item.actorPubkey })
    : item.actorName || "Someone";

  return (
    <button
      aria-label={`Open ${item.title} in ${item.target.project.name}`}
      className={cn(
        "block w-full rounded-xl border border-border/60 bg-card text-left transition-colors hover:bg-muted/20 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
        compact ? "p-3" : "p-4",
      )}
      onClick={onOpen}
      type="button"
    >
      <div className="flex min-w-0 items-start gap-3">
        <UserAvatar
          accent={profile?.isAgent === true}
          avatarUrl={profile?.avatarUrl ?? null}
          className="shrink-0"
          displayName={actorLabel}
          size={compact ? "xs" : "sm"}
        />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 flex-wrap items-baseline gap-x-1.5 gap-y-0.5 text-sm">
            <span className="font-semibold text-foreground">{actorLabel}</span>
            <span className="text-muted-foreground">{item.action}</span>
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="text-xs text-muted-foreground/70">
                  · {relativeTime(item.createdAt)}
                </span>
              </TooltipTrigger>
              <TooltipContent>
                {formatExactTimestamp(item.createdAt)}
              </TooltipContent>
            </Tooltip>
          </div>
          <div className={compact ? "mt-2" : "mt-3"}>
            <div className="flex min-w-0 items-center gap-2">
              <p className="min-w-0 flex-1 truncate text-sm font-semibold text-foreground">
                {item.title}
              </p>
              {item.detail ? (
                <span className="shrink-0 rounded-full border border-border/60 px-2 py-0.5 text-2xs font-medium text-muted-foreground">
                  {item.detail}
                </span>
              ) : null}
              <span
                className={cn(
                  "flex h-6 w-6 shrink-0 items-center justify-center rounded-md",
                  visual.badgeClassName,
                )}
              >
                <Icon className={cn("h-3.5 w-3.5", visual.iconClassName)} />
              </span>
            </div>
            {item.body ? (
              <p
                className={cn(
                  "mt-1.5 text-xs leading-5 text-muted-foreground",
                  compact ? "line-clamp-1" : "line-clamp-2",
                )}
              >
                {item.body}
              </p>
            ) : null}
          </div>
        </div>
      </div>
    </button>
  );
}

/** Mixed GitHub-style workspace activity shown beneath the overview callouts. */
export function ProjectsActivityFeed(props: ProjectsActivityFeedProps) {
  const items = buildActivityItems(props);

  if (props.isLoading && items.length === 0) {
    return (
      <div className={cn(props.compact ? "space-y-2.5" : "space-y-3")}>
        {["first", "second", "third"].map((key) => (
          <div
            className={cn(
              "animate-pulse rounded-xl border border-border/60 bg-muted/20",
              props.compact ? "h-24" : "h-28",
            )}
            key={key}
          />
        ))}
      </div>
    );
  }

  if (items.length === 0) {
    return (
      <div className="rounded-xl border border-dashed border-border/60 px-4 py-12 text-center">
        <p className="text-sm font-medium text-foreground">
          No project activity yet
        </p>
        <p className="mt-1 text-sm text-muted-foreground">
          Commits, pull requests, reviews, and issues will appear here.
        </p>
      </div>
    );
  }

  return (
    <div className={cn(props.compact ? "space-y-2.5" : "space-y-3")}>
      {items.map((item) => (
        <ActivityCard
          compact={props.compact === true}
          item={item}
          key={item.id}
          onOpen={() => {
            if (item.target.type === "project") {
              props.onOpenProject(item.target.project);
            } else if (item.target.type === "pull-request") {
              props.onOpenPullRequest(
                item.target.project,
                item.target.pullRequest,
              );
            } else {
              props.onOpenIssue(item.target.project, item.target.issue);
            }
          }}
          profiles={props.profiles}
        />
      ))}
    </div>
  );
}
