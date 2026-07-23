import { GitPullRequest, MessageSquare } from "lucide-react";

import type {
  Project,
  ProjectPullRequest,
  ProjectPullRequestListItem,
} from "@/features/projects/hooks";
import type { ProjectWorkItemSection } from "@/features/projects/projectWorkItems";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { DropdownMenuItem } from "@/shared/ui/dropdown-menu";
import { ProjectEventTypeIcon } from "./ProjectEventTypeIcon";
import { ProjectListRowMenu } from "./ProjectListRowMenu";
import { ProjectsWorkItemsLoadNotice } from "./ProjectsWorkItemsLoadNotice";
import {
  PROJECT_LIST_CONTAINER_CLASS,
  PROJECT_LIST_ROW_CLASS,
  PROJECT_LIST_ROW_CONTENT_CLASS,
  PROJECT_LIST_ROW_DATE_CLASS,
  PROJECT_LIST_ROW_STATUS_CLASS,
  PROJECT_LIST_ROW_SUBTEXT_CLASS,
  PROJECT_LIST_ROW_TITLE_CLASS,
  PROJECT_LIST_ROW_TRAILING_CLASS,
} from "./projectListRowStyles";

/** Author name that opens the user profile popover. */
function AuthorNameButton({
  label,
  pubkey,
}: {
  label: string;
  pubkey: string;
}) {
  return (
    <UserProfilePopover pubkey={pubkey} triggerElement="span">
      <button
        className="relative z-10 rounded-sm hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
        type="button"
      >
        {label}
      </button>
    </UserProfilePopover>
  );
}

type ProjectsPullRequestsListProps = {
  error: unknown;
  failedSections: ProjectWorkItemSection[];
  isLoading: boolean;
  isRetrying: boolean;
  onOpen: (project: Project, pullRequest: ProjectPullRequest) => void;
  onRetry: () => void;
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
          <ProjectEventTypeIcon className="h-5 w-5" kind="pull-request" />
          <div className="min-w-0 flex-1 space-y-1">
            <div className="flex min-w-0 items-center gap-1.5">
              <p className="truncate text-sm font-semibold text-foreground">
                {pullRequest.title}
              </p>
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
            variant="outline"
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
            <span>created {formatRelativeTime(pullRequest.createdAt)}</span>
            <span>
              by{" "}
              <AuthorNameButton
                label={authorLabel}
                pubkey={pullRequest.author}
              />
            </span>
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

  return (
    <div
      className={PROJECT_LIST_ROW_CLASS}
      data-testid={`projects-pr-row-${pullRequest.id}`}
    >
      <button
        className="absolute inset-0"
        onClick={() => onOpen(project, pullRequest)}
        type="button"
      >
        <span className="sr-only">View {pullRequest.title}</span>
      </button>
      <div className={PROJECT_LIST_ROW_CONTENT_CLASS}>
        <ProjectEventTypeIcon className="h-5 w-5" kind="pull-request" />
        <div className="-mt-0.5 min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-1.5">
            <p className={PROJECT_LIST_ROW_TITLE_CLASS}>{pullRequest.title}</p>
          </div>
          <div
            className={`flex min-w-0 items-center gap-x-1.5 overflow-hidden whitespace-nowrap ${PROJECT_LIST_ROW_SUBTEXT_CLASS}`}
          >
            <span>{project.name}</span>
            <span>·</span>
            <span className="font-mono text-foreground">
              #{pullRequest.id.slice(0, 8)}
            </span>
            <span>
              by{" "}
              <AuthorNameButton
                label={authorLabel}
                pubkey={pullRequest.author}
              />
            </span>
            <span className="md:hidden">·</span>
            <span className="md:hidden">{pullRequest.status}</span>
          </div>
        </div>
        <div className={PROJECT_LIST_ROW_TRAILING_CLASS}>
          <span className={PROJECT_LIST_ROW_STATUS_CLASS}>
            {pullRequest.status}
          </span>
          <div className="hidden w-14 shrink-0 justify-end md:flex">
            {pullRequest.comments.length > 0 ? (
              <span className="flex items-center gap-1 text-2xs leading-3 text-muted-foreground">
                <MessageSquare className="h-3.5 w-3.5" />
                {pullRequest.comments.length}
              </span>
            ) : null}
          </div>
          <span
            className={PROJECT_LIST_ROW_DATE_CLASS}
            data-testid="projects-row-date"
            title={new Date(pullRequest.createdAt * 1_000).toLocaleString()}
          >
            {formatRelativeTime(pullRequest.createdAt)}
          </span>
          <ProjectListRowMenu label={`More options for ${pullRequest.title}`}>
            <DropdownMenuItem onSelect={() => onOpen(project, pullRequest)}>
              <GitPullRequest className="h-4 w-4" />
              {nextStepLabel(pullRequest.status)}
            </DropdownMenuItem>
          </ProjectListRowMenu>
        </div>
      </div>
    </div>
  );
}

export function ProjectsPullRequestsList({
  error,
  failedSections,
  isLoading,
  isRetrying,
  onOpen,
  onRetry,
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

  const loadNotice = (
    <ProjectsWorkItemsLoadNotice
      error={error}
      failedSections={failedSections}
      isRetrying={isRetrying}
      onRetry={onRetry}
      subject="pull requests"
    />
  );

  if (error && pullRequests.length === 0) {
    return loadNotice;
  }

  if (pullRequests.length === 0) {
    return (
      <div className="space-y-3">
        {loadNotice}
        <div className="border border-dashed border-border/60 px-4 py-12 text-center text-sm text-muted-foreground">
          No pull requests yet.
        </div>
      </div>
    );
  }

  if (viewMode === "grid") {
    return (
      <div className="space-y-3">
        {loadNotice}
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
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {loadNotice}
      <div className={PROJECT_LIST_CONTAINER_CLASS}>
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
    </div>
  );
}
