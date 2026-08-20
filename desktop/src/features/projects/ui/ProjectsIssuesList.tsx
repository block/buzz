import { Eye, MessageSquare } from "lucide-react";

import type {
  Project,
  ProjectIssue,
  ProjectIssueListItem,
  Repository,
} from "@/features/projects/hooks";
import { issueShareLink } from "@/features/projects/lib/projectShareLinks";
import {
  listRowDescription,
  relativeTime,
} from "@/features/projects/lib/projectsViewHelpers";
import {
  projectTaskCategoryLabel,
  projectTaskUserLabels,
} from "@/features/projects/projectTaskCategories";
import type { ProjectWorkItemSection } from "@/features/projects/projectWorkItems";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { BuzzLoadingState } from "@/shared/ui/BuzzLoadingState";
import { Card } from "@/shared/ui/card";
import { DropdownMenuItem } from "@/shared/ui/dropdown-menu";
import { CopyShareLinkMenuItem } from "./CopyShareLinkMenuItem";
import { ProjectAuthorIdentity } from "./ProjectAuthorIdentity";
import { ProjectEntityListRow } from "./ProjectEntityListRow";
import { ProjectEventTypeIcon } from "./ProjectEventTypeIcon";
import { ProjectListRowMenu } from "./ProjectListRowMenu";
import {
  PROJECT_LIST_ROW_SUBTEXT_CLASS,
  PROJECT_LIST_ROW_TITLE_CLASS,
} from "./projectListRowStyles";
import { ProjectsWorkItemsLoadNotice } from "./ProjectsWorkItemsLoadNotice";

type ProjectsIssuesListProps = {
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
    issue: ProjectIssue,
  ) => void;
  onRetry: () => void;
  profiles?: UserProfileLookup;
  issues: ProjectIssueListItem[];
  viewMode: "grid" | "list";
};

function nextStepLabel(status: ProjectIssue["status"]) {
  if (status === "Done" || status === "Closed") return "View task";
  if (status === "In Review") return "Review task";
  if (status === "Triage") return "Triage task";
  return "Open task";
}

function issueLabelsSummary(issue: ProjectIssue) {
  const labels = projectTaskUserLabels(issue.labels);
  const visibleLabels = labels.slice(0, 2);
  if (visibleLabels.length === 0) return null;
  const hiddenCount = labels.length - visibleLabels.length;
  return `${visibleLabels.join(", ")}${hiddenCount > 0 ? ` +${hiddenCount}` : ""}`;
}

function IssueHeader({
  authorTestId,
  issue,
  profiles,
  repository,
}: {
  authorTestId?: string;
  issue: ProjectIssue;
  profiles?: UserProfileLookup;
  repository: Repository;
}) {
  const authorLabel = resolveUserLabel({ profiles, pubkey: issue.author });
  const labelsSummary = issueLabelsSummary(issue);

  return (
    <div className="-mt-0.5 min-w-0 flex-1">
      <div className="flex min-w-0 items-center gap-1.5">
        <p className={PROJECT_LIST_ROW_TITLE_CLASS}>{issue.title}</p>
        <span className="shrink-0 rounded-full border border-border/60 px-1.5 py-0.5 text-2xs font-medium text-muted-foreground">
          {projectTaskCategoryLabel(issue.category)}
        </span>
      </div>
      {/* Flex (not inline flow) so the 20px author avatar cannot grow the
          line box — keeps row heights identical to the PR list. */}
      <div
        className={`flex min-w-0 items-center gap-x-1 overflow-hidden whitespace-nowrap ${PROJECT_LIST_ROW_SUBTEXT_CLASS}`}
      >
        <ProjectAuthorIdentity
          label={authorLabel}
          profiles={profiles}
          pubkey={issue.author}
          testId={authorTestId}
        />
        <span>opened this in</span>
        <span className="truncate">{repository.name}</span>
        {labelsSummary ? (
          <>
            <span>and tagged it</span>
            <span className="truncate">{labelsSummary}</span>
          </>
        ) : null}
        <span className="-ml-1">.</span>
        <span className="md:hidden">It is {issue.status.toLowerCase()}.</span>
      </div>
    </div>
  );
}

function IssueGridCard({
  issue,
  onOpen,
  profiles,
  project,
  repository,
}: {
  issue: ProjectIssue;
  onOpen: (project: Project, issue: ProjectIssue) => void;
  profiles?: UserProfileLookup;
  project: Project;
  repository: Repository;
}) {
  const authorLabel = resolveUserLabel({ profiles, pubkey: issue.author });

  return (
    <Card
      className="group relative flex min-h-40 flex-col overflow-hidden border-border/60 bg-transparent p-4 shadow-none transition-colors duration-150 hover:bg-muted/20"
      data-projects-grid-card
    >
      <button
        className="absolute inset-0"
        onClick={() => onOpen(project, issue)}
        type="button"
      >
        <span className="sr-only">
          View task {issue.title} by {authorLabel} in {repository.name}
        </span>
      </button>
      <div className="flex min-h-0 flex-1 flex-col gap-3">
        <div className="flex min-w-0 items-start gap-3">
          <ProjectEventTypeIcon className="h-5 w-5" kind="issue" />
          <IssueHeader
            issue={issue}
            profiles={profiles}
            repository={repository}
          />
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
            <span className="font-medium text-foreground">{issue.status}</span>
            <span>created {relativeTime(issue.createdAt)}</span>
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
  repository,
}: {
  issue: ProjectIssue;
  onOpen: (project: Project, issue: ProjectIssue) => void;
  profiles?: UserProfileLookup;
  project: Project;
  repository: Repository;
}) {
  const authorLabel = resolveUserLabel({ profiles, pubkey: issue.author });

  return (
    <ProjectEntityListRow
      affiliation={repository.name}
      count={issue.comments.length}
      dateSeconds={issue.updatedAt}
      dateTestId="projects-row-date"
      description={listRowDescription(issue.content, issue.title)}
      icon={<ProjectEventTypeIcon className="h-4 w-4" kind="issue" />}
      onClick={() => onOpen(project, issue)}
      peopleSlot={
        <ProjectAuthorIdentity
          label={authorLabel}
          labelClassName="sr-only"
          profiles={profiles}
          pubkey={issue.author}
          testId="projects-issue-author"
        />
      }
      testId={`projects-issue-row-${issue.id}`}
      title={issue.title}
      titleAttr={`Open task ${issue.title}`}
      trailing={
        <ProjectListRowMenu label={`More options for ${issue.title}`}>
          <DropdownMenuItem onSelect={() => onOpen(project, issue)}>
            <Eye className="h-4 w-4" />
            {nextStepLabel(issue.status)}
          </DropdownMenuItem>
          <CopyShareLinkMenuItem
            link={issueShareLink(issue)}
            label="Copy task link"
            testId={`projects-issue-copy-link-${issue.id}`}
          />
        </ProjectListRowMenu>
      }
    />
  );
}

export function ProjectsIssuesList({
  embedded,
  emptyMessage = "No tasks yet.",
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
    return <BuzzLoadingState label="Loading tasks" />;
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
        <div
          className={cn(
            "px-4 py-12 text-center text-sm text-muted-foreground",
            !embedded && "border border-dashed border-border/60",
          )}
        >
          {emptyMessage}
        </div>
      </div>
    );
  }

  if (viewMode === "grid") {
    return (
      <div className="space-y-3">
        {loadNotice}
        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          {issues.map(({ project, issue, repository }) => (
            <IssueGridCard
              issue={issue}
              key={`${repository.id}:${issue.id}`}
              onOpen={(selectedProject, selectedIssue) =>
                onOpen(selectedProject, repository, selectedIssue)
              }
              profiles={profiles}
              project={project}
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
      <ul
        className="divide-y divide-border/60 bg-transparent"
        data-testid="projects-list-container"
      >
        {issues.map(({ project, issue, repository }) => (
          <li key={`${repository.id}:${issue.id}`}>
            <IssueListRow
              issue={issue}
              onOpen={(selectedProject, selectedIssue) =>
                onOpen(selectedProject, repository, selectedIssue)
              }
              profiles={profiles}
              project={project}
              repository={repository}
            />
          </li>
        ))}
      </ul>
    </div>
  );
}
