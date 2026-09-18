import { FolderKanban, GitPullRequest } from "lucide-react";
import * as React from "react";

import type {
  Project,
  ProjectPullRequest,
  ProjectPullRequestListItem,
  Repository,
} from "@/features/projects/hooks";
import { pullRequestShareLink } from "@/features/projects/lib/projectShareLinks";
import { selectionItemFromReview } from "@/features/projects/lib/projectSelection";
import type { ProjectWorkItemSection } from "@/features/projects/projectWorkItems";
import {
  countGroupedRows,
  sliceGroupedRows,
  useIncrementalMount,
} from "@/shared/hooks/useIncrementalMount";
import { cn } from "@/shared/lib/cn";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { BuzzLoadingState } from "@/shared/ui/BuzzLoadingState";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { DropdownMenuItem } from "@/shared/ui/dropdown-menu";
import { CopyShareLinkMenuItem } from "./CopyShareLinkMenuItem";
import { ProjectAuthorIdentity } from "./ProjectAuthorIdentity";
import { ProjectEntityListRow } from "./ProjectEntityListRow";
import { ProjectEventTypeIcon } from "./ProjectEventTypeIcon";
import { PROJECT_GRID_CARD_BODY_CLASS } from "./projectGridCardStyles";
import { ProjectListRowMenu } from "./ProjectListRowMenu";
import { ProjectSelectableGroup } from "./ProjectSelectableGroup";
import { ProjectPanelState } from "./ProjectPanelState";
import { ProjectsWorkItemsLoadNotice } from "./ProjectsWorkItemsLoadNotice";
import { groupProjectWorkItemsByProject } from "./projectWorkItemGroups";

type ProjectsPullRequestsListProps = {
  /** Render without container chrome — a parent table container provides border and rounding. */
  embedded?: boolean;
  emptyMessage?: string;
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
  return "Open review";
}

const PullRequestGridCard = React.memo(function PullRequestGridCard({
  project,
  pullRequest,
  onOpen,
  repository,
}: {
  project: Project;
  pullRequest: ProjectPullRequest;
  onOpen: (
    project: Project,
    repository: Repository,
    pullRequest: ProjectPullRequest,
  ) => void;
  repository: Repository;
}) {
  return (
    <Card
      className="group relative flex min-h-32 flex-col overflow-hidden border-border/60 bg-transparent p-4 shadow-none transition-colors duration-150 hover:bg-muted/20"
      data-projects-grid-card
    >
      <button
        className="absolute inset-0"
        onClick={() => onOpen(project, repository, pullRequest)}
        type="button"
      >
        <span className="sr-only">View review {pullRequest.title}</span>
      </button>
      <div className="flex min-h-0 flex-1 flex-col gap-2">
        <h3
          className="truncate text-sm font-semibold leading-5 text-foreground"
          data-testid="projects-grid-card-title"
        >
          {pullRequest.title}
        </h3>
        <p
          className={cn(PROJECT_GRID_CARD_BODY_CLASS, "text-muted-foreground")}
          data-testid="projects-grid-card-body"
        >
          {pullRequest.content || "No description provided."}
        </p>
        <div
          className="mt-auto flex items-center gap-1.5 text-xs font-medium text-muted-foreground"
          data-testid="projects-grid-card-indicator"
        >
          <ProjectEventTypeIcon className="h-3.5 w-3.5" kind="pull-request" />
          <span>{pullRequest.status}</span>
        </div>
      </div>
    </Card>
  );
});

function reviewSelectionItem(
  project: Project,
  repository: Repository,
  pullRequest: ProjectPullRequest,
) {
  return selectionItemFromReview({
    author: pullRequest.author,
    channelId: repository.channelId ?? project.projectChannelId,
    id: pullRequest.id,
    shareLink: pullRequestShareLink(pullRequest),
    title: pullRequest.title,
  });
}

// Memoized: these rows render in unbounded lists, and any Projects-view
// state change used to re-render every row. Props are kept identity-stable
// by the list (memoized groups/selection arrays, stable onOpen).
const PullRequestListRow = React.memo(function PullRequestListRow({
  project,
  profiles,
  pullRequest,
  rangeItems,
  repository,
  onOpen,
}: {
  project: Project;
  profiles?: UserProfileLookup;
  pullRequest: ProjectPullRequest;
  rangeItems: ReturnType<typeof reviewSelectionItem>[];
  repository: Repository;
  onOpen: (
    project: Project,
    repository: Repository,
    pullRequest: ProjectPullRequest,
  ) => void;
}) {
  const authorLabel = resolveUserLabel({
    profiles,
    pubkey: pullRequest.author,
  });

  return (
    <ProjectEntityListRow
      affiliation={repository.name}
      count={pullRequest.comments.length}
      dateSeconds={pullRequest.updatedAt}
      dateTestId="projects-row-date"
      icon={null}
      onClick={() => onOpen(project, repository, pullRequest)}
      peopleSlot={
        <ProjectAuthorIdentity
          label={authorLabel}
          labelClassName="sr-only"
          profiles={profiles}
          pubkey={pullRequest.author}
          testId="projects-pr-author"
        />
      }
      selection={{
        item: reviewSelectionItem(project, repository, pullRequest),
        rangeItems,
      }}
      testId={`projects-pr-row-${pullRequest.id}`}
      title={pullRequest.title}
      titleAttr={`Open review ${pullRequest.title}`}
      titleIcon={
        <ProjectEventTypeIcon className="h-3.5 w-3.5" kind="pull-request" />
      }
      trailing={
        <ProjectListRowMenu label={`More options for ${pullRequest.title}`}>
          <DropdownMenuItem
            onSelect={() => onOpen(project, repository, pullRequest)}
          >
            <GitPullRequest className="h-4 w-4" />
            {nextStepLabel(pullRequest.status)}
          </DropdownMenuItem>
          <CopyShareLinkMenuItem
            link={pullRequestShareLink(pullRequest)}
            label="Copy review link"
            testId={`projects-pull-request-copy-link-${pullRequest.id}`}
          />
        </ProjectListRowMenu>
      }
    />
  );
});

export function ProjectsPullRequestsList({
  embedded,
  emptyMessage = "No reviews yet",
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
  // Grouping and per-group selection arrays are identity-stable across
  // re-renders so the memoized rows only re-render when their data changes.
  const allGroups = React.useMemo(
    () =>
      groupProjectWorkItemsByProject(pullRequests).map((group) => ({
        ...group,
        selectionItems: group.rows.map((row) =>
          reviewSelectionItem(row.project, row.repository, row.pullRequest),
        ),
      })),
    [pullRequests],
  );
  // Mount rows progressively: a one-shot mount of hundreds of rows blocked
  // the main thread for over a second on tab entry.
  // Each layout's counter grows only while that layout is active: otherwise
  // a layout switch would find the other counter already grown and mount the
  // whole collection in one commit.
  const mountedRowCount = useIncrementalMount(
    countGroupedRows(allGroups),
    30,
    60,
    viewMode !== "grid",
  );
  const mountedGridCount = useIncrementalMount(
    pullRequests.length,
    30,
    60,
    viewMode === "grid",
  );
  const groups = React.useMemo(
    () => sliceGroupedRows(allGroups, mountedRowCount),
    [allGroups, mountedRowCount],
  );

  if (isLoading) {
    return <BuzzLoadingState label="Loading reviews" />;
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
    return (
      <ProjectPanelState
        action={
          <Button
            disabled={isRetrying}
            onClick={onRetry}
            size="sm"
            variant="outline"
          >
            {isRetrying ? "Retrying..." : "Retry"}
          </Button>
        }
        description={
          error instanceof Error ? error.message : "The relay request failed."
        }
        error
        panel={false}
        title="Could not load reviews"
      />
    );
  }

  if (pullRequests.length === 0) {
    return (
      <div className="space-y-3">
        {loadNotice}
        <ProjectPanelState
          className={cn(!embedded && "border border-dashed border-border/60")}
          panel={false}
          title={emptyMessage}
        />
      </div>
    );
  }

  if (viewMode === "grid") {
    return (
      <div className="space-y-3">
        {loadNotice}
        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          {pullRequests
            .slice(0, mountedGridCount)
            .map(({ project, pullRequest, repository }) => (
              <PullRequestGridCard
                key={`${repository.id}:${pullRequest.id}`}
                onOpen={onOpen}
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
      <div data-testid="projects-list-container">
        {groups.map((group) => {
          const groupSelectionItems = group.selectionItems;
          return (
            <ProjectSelectableGroup
              count={group.rows.length}
              groupKey={group.project.id}
              headerClassName="mx-0 gap-3 px-4"
              headerTestId="projects-review-project-group-header"
              icon={<FolderKanban className="h-4 w-4" />}
              items={groupSelectionItems}
              key={group.project.id}
              label={group.project.name}
              labelTestId="project-review-project"
              testId="projects-review-project-group"
            >
              <ul>
                {group.rows.map(({ project, pullRequest, repository }) => (
                  <li
                    className="[contain-intrinsic-size:auto_3.5rem] [content-visibility:auto]"
                    key={`${repository.id}:${pullRequest.id}`}
                  >
                    <PullRequestListRow
                      onOpen={onOpen}
                      profiles={profiles}
                      project={project}
                      pullRequest={pullRequest}
                      rangeItems={groupSelectionItems}
                      repository={repository}
                    />
                  </li>
                ))}
              </ul>
            </ProjectSelectableGroup>
          );
        })}
      </div>
    </div>
  );
}
