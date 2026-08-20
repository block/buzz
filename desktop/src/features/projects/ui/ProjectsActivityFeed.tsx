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
  Repository,
} from "@/features/projects/hooks";
import {
  formatExactTimestamp,
  markdownToPlainText,
  relativeTime,
} from "@/features/projects/lib/projectsViewHelpers";
import {
  projectPullRequestCommentTimelineKind,
  projectPullRequestEffectiveReviewDecision,
} from "@/features/projects/projectPullRequests.mjs";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { BuzzLoadingState } from "@/shared/ui/BuzzLoadingState";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import {
  PROJECT_EVENT_VISUALS,
  type ProjectEventKind,
} from "./ProjectEventTypeIcon";

type ActivityKind = ProjectEventKind;

type ActivityTarget =
  | { type: "project"; project: Project }
  | { type: "commit"; project: Project; commitHash: string }
  | {
      type: "pull-request";
      project: Project;
      repository: Repository;
      pullRequest: ProjectPullRequest;
    }
  | {
      type: "issue";
      project: Project;
      repository: Repository;
      issue: ProjectIssue;
    };

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

type ProjectActivityGroup = {
  key: string;
  label: string;
  items: ProjectActivityItem[];
};

type ProjectsActivityFeedProps = {
  compact?: boolean;
  isLoading: boolean;
  issues: ProjectIssueListItem[];
  onOpenCommit: (project: Project, commitHash: string) => void;
  onOpenIssue: (
    project: Project,
    repository: Repository,
    issue: ProjectIssue,
  ) => void;
  onOpenProject: (project: Project) => void;
  onOpenPullRequest: (
    project: Project,
    repository: Repository,
    pullRequest: ProjectPullRequest,
  ) => void;
  profiles?: UserProfileLookup;
  projects: Project[];
  pullRequests: ProjectPullRequestListItem[];
  snapshots?: Record<string, ProjectRepoSnapshot>;
};

const ACTIVITY_LIMIT = 30;
const WEEK_SECONDS = 7 * 24 * 60 * 60;

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
    items.push({
      id: `repository:${project.id}`,
      kind: "repository",
      createdAt: project.createdAt,
      actorPubkey: project.owner,
      actorName: null,
      action: "created the repository",
      title: project.name,
      body: contentPreview(project.description),
      detail: null,
      target: { type: "project", project },
    });
  }

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

  for (const { project, pullRequest, repository } of pullRequests) {
    const target = {
      type: "pull-request",
      project,
      pullRequest,
      repository,
    } as const;
    items.push({
      id: `pr:${repository.id}:${pullRequest.id}`,
      kind: "pull-request",
      createdAt: pullRequest.createdAt,
      actorPubkey: pullRequest.author,
      actorName: null,
      action: "opened a review in",
      title: pullRequest.title,
      body: contentPreview(pullRequest.content),
      detail: pullRequest.status,
      target,
    });
    for (const update of pullRequest.updates) {
      items.push({
        id: `pr-update:${repository.id}:${update.id}`,
        kind: "commit",
        createdAt: update.createdAt,
        actorPubkey: update.author,
        actorName: null,
        action: "updated a review in",
        title: pullRequest.title,
        body: contentPreview(update.content),
        detail: update.commit?.slice(0, 7) ?? null,
        target,
      });
    }
    for (const comment of pullRequest.comments) {
      const reviewDecision = projectPullRequestEffectiveReviewDecision(
        pullRequest,
        comment,
      );
      const timelineKind = projectPullRequestCommentTimelineKind(comment);
      const kind =
        reviewDecision === "approved"
          ? "approval"
          : reviewDecision === "changes-requested"
            ? "changes-requested"
            : timelineKind === "review-request"
              ? "review-request"
              : "comment";
      items.push({
        id: `pr-comment:${repository.id}:${comment.id}`,
        kind,
        createdAt: comment.createdAt,
        actorPubkey: comment.author,
        actorName: null,
        action:
          kind === "approval"
            ? "approved a review in"
            : kind === "changes-requested"
              ? "requested changes to a review in"
              : kind === "review-request"
                ? "requested review in"
                : "commented on a review in",
        title: pullRequest.title,
        body: contentPreview(comment.content),
        detail:
          kind === "approval"
            ? "Approved"
            : kind === "changes-requested"
              ? "Changes requested"
              : null,
        target,
      });
    }
  }

  for (const { project, issue, repository } of issues) {
    const target = { type: "issue", project, issue, repository } as const;
    items.push({
      id: `issue:${repository.id}:${issue.id}`,
      kind: "issue",
      createdAt: issue.createdAt,
      actorPubkey: issue.author,
      actorName: null,
      action: "created a task in",
      title: issue.title,
      body: contentPreview(issue.content),
      detail: issue.status,
      target,
    });
    for (const comment of issue.comments) {
      items.push({
        id: `issue-comment:${repository.id}:${comment.id}`,
        kind: "comment",
        createdAt: comment.createdAt,
        actorPubkey: comment.author,
        actorName: null,
        action: "commented on a task in",
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

function startOfWeek(timestamp: number) {
  const date = new Date(timestamp * 1_000);
  date.setHours(0, 0, 0, 0);
  const daysSinceMonday = (date.getDay() + 6) % 7;
  date.setDate(date.getDate() - daysSinceMonday);
  return Math.floor(date.getTime() / 1_000);
}

function groupActivityItems(items: ProjectActivityItem[]) {
  const thisWeek = startOfWeek(Math.floor(Date.now() / 1_000));
  const lastWeek = thisWeek - WEEK_SECONDS;
  const groups: ProjectActivityGroup[] = [
    { key: "this-week", label: "This week", items: [] },
    { key: "last-week", label: "Last week", items: [] },
    { key: "earlier", label: "Earlier", items: [] },
  ];

  for (const item of items) {
    if (item.createdAt >= thisWeek) {
      groups[0].items.push(item);
    } else if (item.createdAt >= lastWeek) {
      groups[1].items.push(item);
    } else {
      groups[2].items.push(item);
    }
  }

  return groups.filter((group) => group.items.length > 0);
}

function ActivityCard({
  compact,
  isFirst,
  isLast,
  item,
  onOpen,
  onOpenProject,
  profiles,
}: {
  compact: boolean;
  isFirst: boolean;
  isLast: boolean;
  item: ProjectActivityItem;
  onOpen: () => void;
  onOpenProject: () => void;
  profiles?: UserProfileLookup;
}) {
  const visual = PROJECT_EVENT_VISUALS[item.kind];
  const TypeIcon = visual.icon;
  const profile = item.actorPubkey
    ? profiles?.[normalizePubkey(item.actorPubkey)]
    : undefined;
  const actorLabel = item.actorPubkey
    ? resolveUserLabel({ profiles, pubkey: item.actorPubkey })
    : item.actorName || "Someone";

  return (
    <div
      className={cn(
        "relative block w-full rounded-xl bg-transparent text-left transition-colors hover:bg-muted/20",
        compact ? "py-3 pr-3" : "py-4 pr-4",
      )}
      data-testid="projects-activity-card"
    >
      <button
        aria-label={`Open ${item.title} in ${item.target.project.name}`}
        className="absolute inset-0 rounded-xl focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        onClick={onOpen}
        type="button"
      />
      <div className="pointer-events-none relative flex min-w-0 items-start gap-3">
        {/* Avatar gutter: a vertical spine runs through the avatar centers
            to connect consecutive cards. Segments extend into the card's
            vertical padding so they meet the neighbouring card's segments;
            the first card has no incoming line and the last no outgoing. */}
        <div
          className={cn(
            "relative flex shrink-0 items-start justify-center self-stretch",
            compact ? "w-5" : "w-9",
          )}
        >
          {isFirst ? null : (
            <span
              aria-hidden="true"
              className={cn(
                "absolute left-1/2 w-px -translate-x-1/2 bg-border/80",
                compact ? "-top-3 h-3" : "-top-4 h-4",
              )}
            />
          )}
          {isLast ? null : (
            <span
              aria-hidden="true"
              className={cn(
                "absolute left-1/2 w-px -translate-x-1/2 bg-border/80",
                compact ? "-bottom-3 top-5" : "-bottom-4 top-9",
              )}
            />
          )}
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
              className="relative z-10 shrink-0"
              displayName={actorLabel}
              size={compact ? "xs" : "md"}
            />
          )}
        </div>
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
                      className="pointer-events-auto relative z-10 rounded-sm font-semibold text-foreground hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
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
                  className="pointer-events-auto relative z-10 inline-block max-w-48 truncate rounded-sm align-bottom font-semibold text-foreground hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring sm:max-w-64 2xl:max-w-none"
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
            {/* Bare event-type glyph beside the headline (no badge circle). */}
            <div className="flex min-w-0 items-start gap-2">
              <TypeIcon
                aria-hidden="true"
                className={cn("mt-0.5 h-4 w-4 shrink-0", visual.iconClassName)}
              />
              <p className="min-w-0 flex-1 truncate text-sm font-semibold leading-5 text-foreground">
                {item.title}
              </p>
            </div>
            {item.body ? (
              <p
                className={cn(
                  "mt-0.5 text-sm leading-6 text-muted-foreground",
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
  const groups = groupActivityItems(items);

  if (props.isLoading && items.length === 0) {
    return <BuzzLoadingState label="Loading project activity" />;
  }

  if (items.length === 0) {
    return (
      <div className="rounded-xl border border-dashed border-border/60 px-4 py-12 text-center">
        <p className="text-sm font-medium text-foreground">
          No project activity yet
        </p>
        <p className="mt-1 text-sm text-muted-foreground">
          Commits, reviews, review decisions, and tasks will appear here.
        </p>
      </div>
    );
  }

  return (
    <div
      className="relative space-y-7 bg-transparent"
      data-testid="projects-activity-timeline"
    >
      {groups.map((group) => (
        <section data-testid="projects-activity-group" key={group.key}>
          <div className="mb-1 flex items-center gap-3">
            <h3 className="shrink-0 text-xs font-medium text-muted-foreground">
              {group.label}
            </h3>
            <span aria-hidden="true" className="h-px flex-1 bg-border/70" />
          </div>
          <div>
            {group.items.map((item, index) => {
              return (
                <div className="relative" key={item.id}>
                  <ActivityCard
                    compact={props.compact === true}
                    isFirst={index === 0}
                    isLast={index === group.items.length - 1}
                    item={item}
                    onOpen={() => {
                      if (item.target.type === "project") {
                        props.onOpenProject(item.target.project);
                      } else if (item.target.type === "commit") {
                        props.onOpenCommit(
                          item.target.project,
                          item.target.commitHash,
                        );
                      } else if (item.target.type === "pull-request") {
                        props.onOpenPullRequest(
                          item.target.project,
                          item.target.repository,
                          item.target.pullRequest,
                        );
                      } else {
                        props.onOpenIssue(
                          item.target.project,
                          item.target.repository,
                          item.target.issue,
                        );
                      }
                    }}
                    onOpenProject={() =>
                      props.onOpenProject(item.target.project)
                    }
                    profiles={props.profiles}
                  />
                </div>
              );
            })}
          </div>
        </section>
      ))}
    </div>
  );
}
