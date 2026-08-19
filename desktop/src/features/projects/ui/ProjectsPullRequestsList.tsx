import { GitPullRequest, MessageSquare } from "lucide-react";

import type {
  Project,
  ProjectPullRequest,
  ProjectPullRequestListItem,
  Repository,
} from "@/features/projects/hooks";
import { pullRequestShareLink } from "@/features/projects/lib/projectShareLinks";
import { relativeTime } from "@/features/projects/lib/projectsViewHelpers";
import type { ProjectWorkItemSection } from "@/features/projects/projectWorkItems";
import { cn } from "@/shared/lib/cn";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { DropdownMenuItem } from "@/shared/ui/dropdown-menu";
import { CopyShareLinkMenuItem } from "./CopyShareLinkMenuItem";
import { ProjectAuthorIdentity } from "./ProjectAuthorIdentity";
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

type ProjectsPullRequestsListProps = {
  /** Render without container chrome — a parent table container provides border and rounding. */
  embedded?: boolean;
  error: unknown;
  failedSections: ProjectWorkItemSection[];
  isLoading: boolean;
  isRetrying: boolean;
  onOpen: (
    project: Project,
    repository: Repository,
    pullRequest: ProjectPullRequest,
  ) => void;
  onRetry: () => void;
  profiles?: UserProfileLookup;
  pullRequests: ProjectPullRequestListItem[];
  viewMode: "grid" | "list";
};

function nextStepLabel(status: ProjectPullRequest["status"]) {
  if (status === "Draft") return "View draft";
  if (status === "Merged") return "View merge";
  if (status === "Closed") return "View closed";
  return "Review PR";
}

function PullRequestContext({
  authorLabel,
  authorTestId,
  className,
  profiles,
  pullRequest,
  repository,
  showMobileStatus = false,
}: {
  authorLabel: string;
  authorTestId?: string;
  className?: string;
  profiles?: UserProfileLookup;
  pullRequest: ProjectPullRequest;
  repository: Repository;
  showMobileStatus?: boolean;
}) {
  return (
    <div
      className={cn(
        "flex min-w-0 items-center gap-x-1 overflow-hidden whitespace-nowrap",
        className,
      )}
    >
      <ProjectAuthorIdentity
        label={authorLabel}
        profiles={profiles}
        pubkey={pullRequest.author}
        testId={authorTestId}
      />
      <span>opened this in</span>
      <span className="truncate">{repository.name}</span>
      {pullRequest.branchName && pullRequest.targetBranch ? (
        <>
          <span>to merge</span>
          <span className="truncate">{pullRequest.branchName}</span>
          <span>into</span>
          <span className="truncate">{pullRequest.targetBranch}</span>
        </>
      ) : pullRequest.branchName ? (
        <>
          <span>from</span>
          <span className="truncate">{pullRequest.branchName}</span>
        </>
      ) : pullRequest.targetBranch ? (
        <>
          <span>targeting</span>
          <span className="truncate">{pullRequest.targetBranch}</span>
        </>
      ) : null}
      <span className="-ml-1">.</span>
      {showMobileStatus ? (
        <span className="md:hidden">
          It is {pullRequest.status.toLowerCase()}.
        </span>
      ) : null}
    </div>
  );
}

function PullRequestGridCard({
  project,
  profiles,
  pullRequest,
  repository,
  onOpen,
}: {
  project: Project;
  profiles?: UserProfileLookup;
  pullRequest: ProjectPullRequest;
  repository: Repository;
  onOpen: (project: Project, pullRequest: ProjectPullRequest) => void;
}) {
  const authorLabel = resolveUserLabel({
    profiles,
    pubkey: pullRequest.author,
  });

  return (
    <Card
      className="group relative flex min-h-40 flex-col overflow-hidden border-border/60 bg-transparent p-4 shadow-none transition-colors duration-150 hover:bg-muted/20"
      data-projects-grid-card
    >
      <button
        className="absolute inset-0"
        onClick={() => onOpen(project, pullRequest)}
        type="button"
      >
        <span className="sr-only">
          View pull request {pullRequest.title} by {authorLabel} in{" "}
          {repository.name}
        </span>
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
            <PullRequestContext
              authorLabel={authorLabel}
              className="text-xs leading-4 text-muted-foreground"
              profiles={profiles}
              pullRequest={pullRequest}
              repository={repository}
            />
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
            <span className="font-medium text-foreground">
              {pullRequest.status}
            </span>
            <span>created {relativeTime(pullRequest.createdAt)}</span>
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
  repository,
  onOpen,
}: {
  project: Project;
  profiles?: UserProfileLookup;
  pullRequest: ProjectPullRequest;
  repository: Repository;
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
        <span className="sr-only">
          View pull request {pullRequest.title} by {authorLabel} in{" "}
          {repository.name}
        </span>
      </button>
      <div className={PROJECT_LIST_ROW_CONTENT_CLASS}>
        <ProjectEventTypeIcon className="h-5 w-5" kind="pull-request" />
        <div className="-mt-0.5 min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-1.5">
            <p className={PROJECT_LIST_ROW_TITLE_CLASS}>{pullRequest.title}</p>
          </div>
          <PullRequestContext
            authorLabel={authorLabel}
            authorTestId="projects-pr-author"
            className={PROJECT_LIST_ROW_SUBTEXT_CLASS}
            profiles={profiles}
            pullRequest={pullRequest}
            repository={repository}
            showMobileStatus
          />
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
            {relativeTime(pullRequest.createdAt)}
          </span>
          <ProjectListRowMenu label={`More options for ${pullRequest.title}`}>
            <DropdownMenuItem onSelect={() => onOpen(project, pullRequest)}>
              <GitPullRequest className="h-4 w-4" />
              {nextStepLabel(pullRequest.status)}
            </DropdownMenuItem>
            <CopyShareLinkMenuItem
              link={pullRequestShareLink(pullRequest)}
              testId={`projects-pull-request-copy-link-${pullRequest.id}`}
            />
          </ProjectListRowMenu>
        </div>
      </div>
    </div>
  );
}

export function ProjectsPullRequestsList({
  embedded,
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
      <div
        className={cn(
          "px-4 py-12 text-center text-sm text-muted-foreground",
          !embedded && "border border-border/60",
        )}
      >
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
        <div
          className={cn(
            "px-4 py-12 text-center text-sm text-muted-foreground",
            !embedded && "border border-dashed border-border/60",
          )}
        >
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
          {pullRequests.map(({ project, pullRequest, repository }) => (
            <PullRequestGridCard
              key={`${repository.id}:${pullRequest.id}`}
              onOpen={(selectedProject, selectedPullRequest) =>
                onOpen(selectedProject, repository, selectedPullRequest)
              }
              profiles={profiles}
              project={project}
              pullRequest={pullRequest}
              repository={repository}
            />
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {loadNotice}
      <div
        className={
          embedded ? "divide-y divide-border/60" : PROJECT_LIST_CONTAINER_CLASS
        }
        data-testid="projects-list-container"
      >
        {pullRequests.map(({ project, pullRequest, repository }) => (
          <PullRequestListRow
            key={`${repository.id}:${pullRequest.id}`}
            onOpen={(selectedProject, selectedPullRequest) =>
              onOpen(selectedProject, repository, selectedPullRequest)
            }
            profiles={profiles}
            project={project}
            pullRequest={pullRequest}
            repository={repository}
          />
        ))}
      </div>
    </div>
  );
}
