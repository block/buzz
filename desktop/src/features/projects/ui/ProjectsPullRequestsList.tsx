import { Check, MessageSquare, X } from "lucide-react";

import type {
  Project,
  ProjectPullRequest,
  ProjectPullRequestListItem,
} from "@/features/projects/hooks";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type ProjectsPullRequestsListProps = {
  isLoading: boolean;
  onOpen: (project: Project, pullRequest: ProjectPullRequest) => void;
  profiles?: UserProfileLookup;
  pullRequests: ProjectPullRequestListItem[];
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

function pullRequestStatusClassName(status: ProjectPullRequest["status"]) {
  if (status === "Closed") return "text-destructive";
  if (status === "Draft") return "text-muted-foreground";
  if (status === "Merged") return "text-purple-400";
  return "text-green-500";
}

function nextStepLabel(status: ProjectPullRequest["status"]) {
  if (status === "Draft") return "View draft";
  if (status === "Merged") return "View merge";
  if (status === "Closed") return "View closed";
  return "Review PR";
}

function PullRequestGridCard({
  project,
  profiles,
  pullRequest,
  onOpen,
}: {
  project: Project;
  profiles?: UserProfileLookup;
  pullRequest: ProjectPullRequest;
  onOpen: (project: Project, pullRequest: ProjectPullRequest) => void;
}) {
  const authorLabel = resolveUserLabel({
    profiles,
    pubkey: pullRequest.author,
  });
  const authorProfile = profiles?.[normalizePubkey(pullRequest.author)];
  const StatusIcon =
    pullRequest.status === "Closed" || pullRequest.status === "Draft"
      ? X
      : Check;

  return (
    <Card className="group relative flex min-h-40 flex-col overflow-hidden border-border/60 bg-card p-4 shadow-none transition-colors duration-150 hover:bg-muted/20">
      <button
        className="absolute inset-0"
        onClick={() => onOpen(project, pullRequest)}
        type="button"
      >
        <span className="sr-only">View {pullRequest.title}</span>
      </button>
      <div className="flex min-h-0 flex-1 flex-col gap-3">
        <div className="flex min-w-0 items-start gap-3">
          <Tooltip>
            <TooltipTrigger asChild>
              <span className="relative z-10 inline-flex shrink-0">
                <UserAvatar
                  accent={authorProfile?.isAgent === true}
                  avatarUrl={authorProfile?.avatarUrl ?? null}
                  displayName={authorLabel}
                  size="md"
                />
              </span>
            </TooltipTrigger>
            <TooltipContent>{authorLabel}</TooltipContent>
          </Tooltip>
          <div className="min-w-0 flex-1 space-y-1">
            <div className="flex min-w-0 items-center gap-1.5">
              <p className="truncate text-sm font-semibold text-foreground">
                {pullRequest.title}
              </p>
              <StatusIcon
                className={`h-3.5 w-3.5 shrink-0 ${pullRequestStatusClassName(
                  pullRequest.status,
                )}`}
              />
            </div>
            <p className="truncate text-xs text-muted-foreground">
              {project.name}
            </p>
          </div>
          <Button
            className="relative z-10 h-7 shrink-0 px-2.5"
            onClick={(event) => {
              event.stopPropagation();
              onOpen(project, pullRequest);
            }}
            size="xs"
            type="button"
            variant="default"
          >
            {nextStepLabel(pullRequest.status)}
          </Button>
        </div>

        {pullRequest.content ? (
          <p className="line-clamp-2 text-sm text-foreground/90">
            {pullRequest.content}
          </p>
        ) : null}

        <div className="mt-auto border border-border/60 bg-muted/30 px-2.5 py-2">
          <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs text-foreground/80">
            <span className="font-mono text-foreground">
              #{pullRequest.id.slice(0, 8)}
            </span>
            <span className="font-medium text-foreground">
              {pullRequest.status}
            </span>
            <span>opened {formatRelativeTime(pullRequest.createdAt)}</span>
            <span>by {authorLabel}</span>
            {pullRequest.comments.length > 0 ? (
              <span className="flex items-center gap-1">
                <MessageSquare className="h-3.5 w-3.5" />
                {pullRequest.comments.length}
              </span>
            ) : null}
          </div>
        </div>
      </div>
    </Card>
  );
}

function PullRequestListRow({
  project,
  profiles,
  pullRequest,
  onOpen,
}: {
  project: Project;
  profiles?: UserProfileLookup;
  pullRequest: ProjectPullRequest;
  onOpen: (project: Project, pullRequest: ProjectPullRequest) => void;
}) {
  const authorLabel = resolveUserLabel({
    profiles,
    pubkey: pullRequest.author,
  });
  const authorProfile = profiles?.[normalizePubkey(pullRequest.author)];
  const StatusIcon =
    pullRequest.status === "Closed" || pullRequest.status === "Draft"
      ? X
      : Check;

  return (
    <div className="group relative px-4 py-2.5 transition-colors duration-150 hover:bg-muted/20">
      <button
        className="absolute inset-0"
        onClick={() => onOpen(project, pullRequest)}
        type="button"
      >
        <span className="sr-only">View {pullRequest.title}</span>
      </button>
      <div className="flex min-w-0 items-start gap-3">
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="relative z-10 inline-flex shrink-0">
              <UserAvatar
                accent={authorProfile?.isAgent === true}
                avatarUrl={authorProfile?.avatarUrl ?? null}
                displayName={authorLabel}
                size="md"
              />
            </span>
          </TooltipTrigger>
          <TooltipContent>{authorLabel}</TooltipContent>
        </Tooltip>
        <div className="min-w-0 flex-1 space-y-1">
          <div className="flex min-w-0 items-center gap-1.5">
            <p className="truncate text-sm font-semibold text-foreground">
              {pullRequest.title}
            </p>
            <StatusIcon
              className={`h-3.5 w-3.5 shrink-0 ${pullRequestStatusClassName(
                pullRequest.status,
              )}`}
            />
          </div>
          <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs text-foreground/80">
            <span>{project.name}</span>
            <span>·</span>
            <span className="font-mono text-foreground">
              #{pullRequest.id.slice(0, 8)}
            </span>
            <span>opened {formatRelativeTime(pullRequest.createdAt)}</span>
            <span>by {authorLabel}</span>
            <span>·</span>
            <span>{pullRequest.status}</span>
          </div>
        </div>
        <div className="relative z-10 flex shrink-0 items-center gap-2">
          {pullRequest.comments.length > 0 ? (
            <span className="flex items-center gap-1 text-xs text-muted-foreground">
              <MessageSquare className="h-3.5 w-3.5" />
              {pullRequest.comments.length}
            </span>
          ) : null}
          <Button
            className="h-7 px-2.5"
            onClick={(event) => {
              event.stopPropagation();
              onOpen(project, pullRequest);
            }}
            size="xs"
            type="button"
            variant="default"
          >
            {nextStepLabel(pullRequest.status)}
          </Button>
        </div>
      </div>
    </div>
  );
}

export function ProjectsPullRequestsList({
  isLoading,
  onOpen,
  profiles,
  pullRequests,
  viewMode,
}: ProjectsPullRequestsListProps) {
  if (isLoading) {
    return (
      <div className="border border-border/60 px-4 py-12 text-center text-sm text-muted-foreground">
        Loading pull requests...
      </div>
    );
  }

  if (pullRequests.length === 0) {
    return (
      <div className="border border-dashed border-border/60 px-4 py-12 text-center text-sm text-muted-foreground">
        No pull requests yet.
      </div>
    );
  }

  if (viewMode === "grid") {
    return (
      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
        {pullRequests.map(({ project, pullRequest }) => (
          <PullRequestGridCard
            key={pullRequest.id}
            onOpen={onOpen}
            profiles={profiles}
            project={project}
            pullRequest={pullRequest}
          />
        ))}
      </div>
    );
  }

  return (
    <div className="-mx-4 divide-y divide-border/60 border-y border-border/60 bg-card">
      {pullRequests.map(({ project, pullRequest }) => (
        <PullRequestListRow
          key={pullRequest.id}
          onOpen={onOpen}
          profiles={profiles}
          project={project}
          pullRequest={pullRequest}
        />
      ))}
    </div>
  );
}
