import { Clock3, Hash, MessageSquare, Tag, Users } from "lucide-react";

import type {
  Project,
  ProjectIssue,
  ProjectIssueListItem,
} from "@/features/projects/hooks";
import {
  groupProjectIssuesForBoard,
  normalizeProjectIssueBoardStatus,
} from "@/features/projects/lib/projectIssueBoard.mjs";
import { relativeTime } from "@/features/projects/lib/projectsViewHelpers";
import type { ProjectWorkItemSection } from "@/features/projects/projectWorkItems";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { Card } from "@/shared/ui/card";
import { ProjectsWorkItemsLoadNotice } from "./ProjectsWorkItemsLoadNotice";

type ProjectsIssuesBoardProps = {
  error: unknown;
  failedSections: ProjectWorkItemSection[];
  isLoading: boolean;
  isRetrying: boolean;
  issues: ProjectIssueListItem[];
  onOpen: (project: Project, issue: ProjectIssue) => void;
  onRetry: () => void;
  profiles?: UserProfileLookup;
};

const UUID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

function IssueBoardCard({
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
  const people = [...new Set([issue.author, ...issue.recipients])];
  const peopleLabel = people
    .map((pubkey) => resolveUserLabel({ profiles, pubkey }))
    .join(", ");
  const visibleLabels = issue.labels.slice(0, 2);
  const hiddenLabelCount = issue.labels.length - visibleLabels.length;
  const hasChannelBinding =
    project.projectChannelId !== null &&
    UUID_PATTERN.test(project.projectChannelId);

  return (
    <Card
      asChild
      className="rounded-lg border-border/60 bg-card/70 shadow-none transition-colors duration-150 hover:bg-muted/30 focus-within:border-ring"
    >
      <button
        aria-label={`Open ${issue.title} in ${project.name}`}
        className="block w-full p-3 text-left focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        data-testid={`projects-issue-board-card-${issue.id}`}
        onClick={() => onOpen(project, issue)}
        type="button"
      >
        <div className="flex items-start justify-between gap-2">
          <span className="line-clamp-2 text-sm font-medium leading-5 text-foreground">
            {issue.title}
          </span>
          <span className="shrink-0 rounded-md border border-border/60 bg-muted/30 px-1.5 py-0.5 text-2xs font-medium text-muted-foreground">
            {normalizeProjectIssueBoardStatus(issue.status)}
          </span>
        </div>

        <p className="mt-1 truncate text-xs text-muted-foreground">
          {project.name}
        </p>

        {visibleLabels.length > 0 ? (
          <div
            className="mt-2 flex min-w-0 flex-wrap items-center gap-1"
            title={`Labels: ${issue.labels.join(", ")}`}
          >
            <Tag className="h-3 w-3 shrink-0 text-muted-foreground" />
            {visibleLabels.map((label) => (
              <span
                className="max-w-24 truncate rounded-md bg-muted/50 px-1.5 py-0.5 text-2xs text-muted-foreground"
                key={label}
              >
                {label}
              </span>
            ))}
            {hiddenLabelCount > 0 ? (
              <span className="text-2xs text-muted-foreground">
                +{hiddenLabelCount}
              </span>
            ) : null}
          </div>
        ) : null}

        <div className="mt-3 flex flex-wrap items-center gap-x-2.5 gap-y-1 border-t border-border/50 pt-2 text-2xs text-muted-foreground">
          <span className="flex items-center gap-1" title={peopleLabel}>
            <Users className="h-3 w-3" />
            {people.length}
            <span className="sr-only">
              {people.length === 1 ? "person" : "people"}
            </span>
          </span>
          <span className="flex items-center gap-1">
            <MessageSquare className="h-3 w-3" />
            {issue.comments.length}
            <span className="sr-only">
              {issue.comments.length === 1 ? "comment" : "comments"}
            </span>
          </span>
          <span
            className="flex items-center gap-1"
            title={new Date(issue.updatedAt * 1_000).toLocaleString()}
          >
            <Clock3 className="h-3 w-3" />
            {relativeTime(issue.updatedAt)}
          </span>
        </div>

        <div className="mt-1.5 flex min-w-0 items-center justify-between gap-2 text-2xs text-muted-foreground">
          <span className="flex items-center gap-0.5 font-mono">
            <Hash className="h-3 w-3" />
            {issue.id.slice(0, 8)}
          </span>
          {hasChannelBinding ? (
            <span
              className="max-w-28 truncate"
              title={`Project channel ${project.projectChannelId}`}
            >
              Channel {project.projectChannelId?.slice(0, 8)}
            </span>
          ) : null}
        </div>
      </button>
    </Card>
  );
}

export function ProjectsIssuesBoard({
  error,
  failedSections,
  isLoading,
  isRetrying,
  issues,
  onOpen,
  onRetry,
  profiles,
}: ProjectsIssuesBoardProps) {
  if (isLoading) {
    return (
      <div className="border border-border/60 px-4 py-12 text-center text-sm text-muted-foreground">
        Loading issue board...
      </div>
    );
  }

  const loadNotice = (
    <ProjectsWorkItemsLoadNotice
      error={error}
      failedSections={failedSections}
      isRetrying={isRetrying}
      onRetry={onRetry}
      subject="issues"
    />
  );

  if (error && issues.length === 0) {
    return loadNotice;
  }

  if (issues.length === 0) {
    return (
      <div className="space-y-3">
        {loadNotice}
        <div className="border border-dashed border-border/60 px-4 py-12 text-center text-sm text-muted-foreground">
          No issues yet.
        </div>
      </div>
    );
  }

  const columns = groupProjectIssuesForBoard(issues);

  return (
    <div className="space-y-3">
      {loadNotice}
      <section
        aria-label="Issues board"
        className="max-w-full overflow-x-auto pb-2"
        data-testid="projects-issues-board"
      >
        <div className="grid min-w-max grid-flow-col auto-cols-72 gap-3">
          {columns.map((column) => {
            const columnId = `issue-board-${column.status
              .toLowerCase()
              .replaceAll(" ", "-")}`;
            return (
              <section
                aria-labelledby={columnId}
                className="w-72 rounded-xl border border-border/60 bg-muted/15 p-2.5"
                key={column.status}
              >
                <div className="mb-2.5 flex items-center justify-between gap-2 px-0.5">
                  <h2
                    className="text-xs font-semibold text-foreground"
                    id={columnId}
                  >
                    {column.status}
                  </h2>
                  <span className="rounded-md bg-muted/60 px-1.5 py-0.5 text-2xs font-medium text-muted-foreground">
                    {column.issues.length}
                    <span className="sr-only">
                      {column.issues.length === 1 ? " issue" : " issues"}
                    </span>
                  </span>
                </div>
                {column.issues.length === 0 ? (
                  <p className="rounded-lg border border-dashed border-border/50 px-3 py-8 text-center text-xs text-muted-foreground">
                    No {column.status.toLowerCase()} issues
                  </p>
                ) : (
                  <ul className="space-y-2">
                    {column.issues.map(({ issue, project }) => (
                      <li key={issue.id}>
                        <IssueBoardCard
                          issue={issue}
                          onOpen={onOpen}
                          profiles={profiles}
                          project={project}
                        />
                      </li>
                    ))}
                  </ul>
                )}
              </section>
            );
          })}
        </div>
      </section>
    </div>
  );
}
