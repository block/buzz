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
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
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
  markdownToPlainText,
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
  | { type: "commit"; project: Project; commitHash: string }
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
  onOpenCommit: (project: Project, commitHash: string) => void;
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
    detailClassName: string;
  }
> = {
  commit: {
    icon: GitCommitHorizontal,
    iconClassName: "text-primary",
    badgeClassName: "bg-primary/10 text-primary",
    detailClassName: "border-primary/30 text-primary",
  },
  "pull-request": {
    icon: GitPullRequest,
    iconClassName: "text-green-600 dark:text-green-500",
    badgeClassName:
      "bg-green-600/10 text-green-700 dark:bg-green-500/10 dark:text-green-400",
    detailClassName:
      "border-green-600/30 text-green-700 dark:border-green-500/30 dark:text-green-400",
  },
  issue: {
    icon: CircleDot,
    iconClassName: "text-orange-500",
    badgeClassName: "bg-orange-500/10 text-orange-700 dark:text-orange-300",
    detailClassName:
      "border-orange-500/30 text-orange-700 dark:text-orange-300",
  },
  comment: {
    icon: MessageSquare,
    iconClassName: "text-muted-foreground",
    badgeClassName: "bg-muted text-muted-foreground",
    detailClassName: "border-border/60 text-muted-foreground",
  },
  approval: {
    icon: Check,
    iconClassName: "text-green-600 dark:text-green-500",
    badgeClassName:
      "bg-green-600/10 text-green-700 dark:bg-green-500/10 dark:text-green-400",
    detailClassName:
      "border-green-600/30 text-green-700 dark:border-green-500/30 dark:text-green-400",
  },
  "review-request": {
    icon: UserPlus,
    iconClassName: "text-blue-600 dark:text-blue-400",
    badgeClassName:
      "bg-blue-600/10 text-blue-700 dark:bg-blue-500/10 dark:text-blue-300",
    detailClassName:
      "border-blue-600/30 text-blue-700 dark:border-blue-500/30 dark:text-blue-300",
  },
};

function contentPreview(content: string) {
  return markdownToPlainText(content).replace(/\s+/g, " ").trim().slice(0, 280);
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
      action: "pushed a commit to",
      title: commit.subject || commit.shortHash,
      body: "",
      detail: commit.shortHash,
      target: {
        type: "commit",
        project,
        commitHash: commit.hash,
      },
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
      action: "opened a pull request in",
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
        action: "updated a pull request in",
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
          ? "approved a pull request in"
          : comment.isReviewRequest
            ? "requested review in"
            : "commented on a pull request in",
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
      action: "created an issue in",
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
        action: "commented on an issue in",
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
  onOpenProject,
  profiles,
}: {
  compact: boolean;
  item: ProjectActivityItem;
  onOpen: () => void;
  onOpenProject: () => void;
  profiles?: UserProfileLookup;
}) {
  const visual = KIND_VISUALS[item.kind];
  const profile = item.actorPubkey
    ? profiles?.[normalizePubkey(item.actorPubkey)]
    : undefined;
  const actorLabel = item.actorPubkey
    ? resolveUserLabel({ profiles, pubkey: item.actorPubkey })
    : item.actorName || "Someone";

  return (
    <div
      className={cn(
        "relative block w-full rounded-xl border border-border/60 bg-card text-left transition-colors hover:bg-muted/20",
        compact ? "p-3" : "p-4",
      )}
    >
      <button
        aria-label={`Open ${item.title} in ${item.target.project.name}`}
        className="absolute inset-0 rounded-xl focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        onClick={onOpen}
        type="button"
      />
      <div className="pointer-events-none relative flex min-w-0 items-start gap-3">
        {item.actorPubkey ? (
          <UserProfilePopover pubkey={item.actorPubkey} triggerElement="span">
            <button
              aria-label={`View ${actorLabel}'s profile`}
              className="pointer-events-auto relative z-10 shrink-0 rounded-full focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
              type="button"
            >
              <UserAvatar
                accent={profile?.isAgent === true}
                avatarUrl={profile?.avatarUrl ?? null}
                displayName={actorLabel}
                size={compact ? "xs" : "md"}
              />
            </button>
          </UserProfilePopover>
        ) : (
          <UserAvatar
            accent={profile?.isAgent === true}
            avatarUrl={profile?.avatarUrl ?? null}
            className="shrink-0"
            displayName={actorLabel}
            size={compact ? "xs" : "md"}
          />
        )}
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-start gap-2">
            <div className="min-w-0 flex-1 text-xs text-muted-foreground/70">
              <span>
                {item.actorPubkey ? (
                  <UserProfilePopover
                    pubkey={item.actorPubkey}
                    triggerElement="span"
                  >
                    <button
                      className="pointer-events-auto relative z-10 rounded-sm hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
                      type="button"
                    >
                      {actorLabel}
                    </button>
                  </UserProfilePopover>
                ) : (
                  actorLabel
                )}{" "}
                {item.action}{" "}
                <button
                  className="pointer-events-auto relative z-10 inline-block max-w-48 truncate rounded-sm align-bottom hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring sm:max-w-64 2xl:max-w-none"
                  onClick={onOpenProject}
                  type="button"
                >
                  {item.target.project.name}
                </button>
              </span>
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    className="pointer-events-auto relative z-10 mt-0.5 block w-fit rounded-sm hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
                    onClick={onOpen}
                    type="button"
                  >
                    {relativeTime(item.createdAt)}
                  </button>
                </TooltipTrigger>
                <TooltipContent>
                  {formatExactTimestamp(item.createdAt)}
                </TooltipContent>
              </Tooltip>
            </div>
            {item.detail ? (
              <span
                className={cn(
                  "shrink-0 rounded-full border px-2 py-0.5 text-2xs font-medium",
                  visual.detailClassName,
                )}
              >
                {item.detail}
              </span>
            ) : null}
          </div>
          <div className={compact ? "mt-2" : "mt-3"}>
            <p className="min-w-0 truncate text-sm font-semibold text-foreground">
              {item.title}
            </p>
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
    </div>
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
    <div
      className={cn("relative", props.compact ? "space-y-2.5" : "space-y-3")}
    >
      <div
        aria-hidden="true"
        className="absolute bottom-2 left-[7px] top-2 w-px bg-border/45"
      />
      {items.map((item) => {
        const visual = KIND_VISUALS[item.kind];
        const Icon = visual.icon;
        return (
          <div className="relative pl-7" key={item.id}>
            <span
              aria-hidden="true"
              className={cn(
                "absolute left-2 h-px w-5 bg-border/60",
                props.compact ? "top-[1.375rem]" : "top-[2.125rem]",
              )}
            />
            <span
              aria-hidden="true"
              className={cn(
                "absolute left-0 z-10 flex h-4 w-4 items-center justify-center rounded-full ring-1 ring-border/60",
                props.compact ? "top-3.5" : "top-[1.625rem]",
                visual.badgeClassName,
              )}
            >
              <Icon className={cn("h-3 w-3", visual.iconClassName)} />
            </span>
            <ActivityCard
              compact={props.compact === true}
              item={item}
              onOpen={() => {
                if (item.target.type === "commit") {
                  props.onOpenCommit(
                    item.target.project,
                    item.target.commitHash,
                  );
                } else if (item.target.type === "pull-request") {
                  props.onOpenPullRequest(
                    item.target.project,
                    item.target.pullRequest,
                  );
                } else {
                  props.onOpenIssue(item.target.project, item.target.issue);
                }
              }}
              onOpenProject={() => props.onOpenProject(item.target.project)}
              profiles={props.profiles}
            />
          </div>
        );
      })}
    </div>
  );
}
