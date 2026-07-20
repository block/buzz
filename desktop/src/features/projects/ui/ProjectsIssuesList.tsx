import { Eye, MessageSquare } from "lucide-react";

import type {
  Project,
  ProjectIssue,
  ProjectIssueListItem,
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

type ProjectsIssuesListProps = {
  error: unknown;
  failedSections: ProjectWorkItemSection[];
  isLoading: boolean;
  isRetrying: boolean;
  onOpen: (project: Project, issue: ProjectIssue) => void;
  onRetry: () => void;
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

function nextStepLabel(status: ProjectIssue["status"]) {
  if (status === "Done" || status === "Closed") return "View issue";
  if (status === "In Review") return "Review issue";
  if (status === "Triage") return "Triage issue";
  return "Open issue";
}

function IssueHeader({
  includeDate = true,
  issue,
  profiles,
  project,
}: {
  includeDate?: boolean;
  issue: ProjectIssue;
  profiles?: UserProfileLookup;
  project: Project;
}) {
  const authorLabel = resolveUserLabel({ profiles, pubkey: issue.author });

  return (
    <div className="-mt-0.5 min-w-0 flex-1">
      <div className="flex min-w-0 items-center gap-1.5">
        <p className={PROJECT_LIST_ROW_TITLE_CLASS}>{issue.title}</p>
      </div>
      <p className={`truncate ${PROJECT_LIST_ROW_SUBTEXT_CLASS}`}>
        {project.name}
        {includeDate
          ? ` · created ${formatRelativeTime(issue.createdAt)}`
          : null}{" "}
        · by{" "}
        <UserProfilePopover pubkey={issue.author} triggerElement="span">
          <button
            className="relative z-10 rounded-sm hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
            type="button"
          >
            {authorLabel}
          </button>
        </UserProfilePopover>
        {includeDate ? (
          ` · ${issue.status}`
        ) : (
          <>
            <span className="md:hidden"> · </span>
            <span className="md:hidden">{issue.status}</span>
          </>
        )}
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
          <ProjectEventTypeIcon className="h-5 w-5" kind="issue" />
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
    <div
      className={PROJECT_LIST_ROW_CLASS}
      data-testid={`projects-issue-row-${issue.id}`}
    >
      <button
        className="absolute inset-0"
        onClick={() => onOpen(project, issue)}
        type="button"
      >
        <span className="sr-only">View {issue.title}</span>
      </button>
      <div className={PROJECT_LIST_ROW_CONTENT_CLASS}>
        <ProjectEventTypeIcon className="h-5 w-5" kind="issue" />
        <IssueHeader
          includeDate={false}
          issue={issue}
          profiles={profiles}
          project={project}
        />
        <div className={PROJECT_LIST_ROW_TRAILING_CLASS}>
          <span className={PROJECT_LIST_ROW_STATUS_CLASS}>{issue.status}</span>
          <div className="hidden w-14 shrink-0 justify-end md:flex">
            {issue.comments.length > 0 ? (
              <span className="flex items-center gap-1 text-2xs leading-3 text-muted-foreground">
                <MessageSquare className="h-3.5 w-3.5" />
                {issue.comments.length}
              </span>
            ) : null}
          </div>
          <span
            className={PROJECT_LIST_ROW_DATE_CLASS}
            data-testid="projects-row-date"
            title={new Date(issue.createdAt * 1_000).toLocaleString()}
          >
            {formatRelativeTime(issue.createdAt)}
          </span>
          <ProjectListRowMenu label={`More options for ${issue.title}`}>
            <DropdownMenuItem onSelect={() => onOpen(project, issue)}>
              <Eye className="h-4 w-4" />
              {nextStepLabel(issue.status)}
            </DropdownMenuItem>
          </ProjectListRowMenu>
        </div>
      </div>
    </div>
  );
}

export function ProjectsIssuesList({
  error,
  failedSections,
  isLoading,
  isRetrying,
  issues,
  onOpen,
  onRetry,
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

  if (viewMode === "grid") {
    return (
      <div className="space-y-3">
        {loadNotice}
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
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {loadNotice}
      <div className={PROJECT_LIST_CONTAINER_CLASS}>
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
    </div>
  );
}
