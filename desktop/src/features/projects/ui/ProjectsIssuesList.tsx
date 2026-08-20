import { Eye, FolderKanban } from "lucide-react";

import type {
  Project,
  ProjectIssue,
  ProjectIssueListItem,
  Repository,
} from "@/features/projects/hooks";
import { issueShareLink } from "@/features/projects/lib/projectShareLinks";
import { selectionItemFromTask } from "@/features/projects/lib/projectSelection";
import type { ProjectWorkItemSection } from "@/features/projects/projectWorkItems";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { BuzzLoadingState } from "@/shared/ui/BuzzLoadingState";
import { Card } from "@/shared/ui/card";
import { DropdownMenuItem } from "@/shared/ui/dropdown-menu";
import { CopyShareLinkMenuItem } from "./CopyShareLinkMenuItem";
import { ProjectAuthorIdentity } from "./ProjectAuthorIdentity";
import { ProjectEntityListRow } from "./ProjectEntityListRow";
import { ProjectEventTypeIcon } from "./ProjectEventTypeIcon";
import { PROJECT_GRID_CARD_BODY_CLASS } from "./projectGridCardStyles";
import { ProjectListRowMenu } from "./ProjectListRowMenu";
import { ProjectSelectableGroup } from "./ProjectSelectableGroup";
import { ProjectsWorkItemsLoadNotice } from "./ProjectsWorkItemsLoadNotice";
import { groupProjectWorkItemsByProject } from "./projectWorkItemGroups";

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

function IssueGridCard({
  issue,
  onOpen,
  project,
}: {
  issue: ProjectIssue;
  onOpen: (project: Project, issue: ProjectIssue) => void;
  project: Project;
}) {
  return (
    <Card
      className="group relative flex min-h-32 flex-col overflow-hidden border-border/60 bg-transparent p-4 shadow-none transition-colors duration-150 hover:bg-muted/20"
      data-projects-grid-card
    >
      <button
        className="absolute inset-0"
        onClick={() => onOpen(project, issue)}
        type="button"
      >
        <span className="sr-only">View task {issue.title}</span>
      </button>
      <div className="flex min-h-0 flex-1 flex-col gap-2">
        <h3
          className="truncate text-sm font-semibold leading-5 text-foreground"
          data-testid="projects-grid-card-title"
        >
          {issue.title}
        </h3>
        <p
          className={cn(PROJECT_GRID_CARD_BODY_CLASS, "text-muted-foreground")}
          data-testid="projects-grid-card-body"
        >
          {issue.content || "No description provided."}
        </p>
        <div
          className="mt-auto flex items-center gap-1.5 text-xs font-medium text-muted-foreground"
          data-testid="projects-grid-card-indicator"
        >
          <ProjectEventTypeIcon className="h-3.5 w-3.5" kind="issue" />
          <span>{issue.status}</span>
        </div>
      </div>
    </Card>
  );
}

function issueSelectionItem(
  project: Project,
  repository: Repository,
  issue: ProjectIssue,
) {
  return selectionItemFromTask({
    author: issue.author,
    channelId: repository.channelId ?? project.projectChannelId,
    id: issue.id,
    shareLink: issueShareLink(issue),
    title: issue.title,
  });
}

function IssueListRow({
  issue,
  onOpen,
  profiles,
  project,
  rangeItems,
  repository,
}: {
  issue: ProjectIssue;
  onOpen: (project: Project, issue: ProjectIssue) => void;
  profiles?: UserProfileLookup;
  project: Project;
  rangeItems: ReturnType<typeof issueSelectionItem>[];
  repository: Repository;
}) {
  const authorLabel = resolveUserLabel({ profiles, pubkey: issue.author });

  return (
    <ProjectEntityListRow
      affiliation={repository.name}
      count={issue.comments.length}
      dateSeconds={issue.updatedAt}
      dateTestId="projects-row-date"
      icon={null}
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
      selection={{
        item: issueSelectionItem(project, repository, issue),
        rangeItems,
      }}
      testId={`projects-issue-row-${issue.id}`}
      title={issue.title}
      titleAttr={`Open task ${issue.title}`}
      titleIcon={<ProjectEventTypeIcon className="h-3.5 w-3.5" kind="issue" />}
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
              project={project}
            />
          ))}
        </div>
      </div>
    );
  }

  const groups = groupProjectWorkItemsByProject(issues);

  return (
    <div className="space-y-3">
      {loadNotice}
      <div data-testid="projects-list-container">
        {groups.map((group) => {
          const groupSelectionItems = group.rows.map((row) =>
            issueSelectionItem(row.project, row.repository, row.issue),
          );
          return (
            <ProjectSelectableGroup
              count={group.rows.length}
              groupKey={group.project.id}
              headerTestId="projects-issue-project-group-header"
              icon={<FolderKanban className="h-4 w-4" />}
              items={groupSelectionItems}
              key={group.project.id}
              label={group.project.name}
              labelTestId="project-issue-project"
              testId="projects-issue-project-group"
            >
              <ul>
                {group.rows.map(({ project, issue, repository }) => (
                  <li key={`${repository.id}:${issue.id}`}>
                    <IssueListRow
                      issue={issue}
                      onOpen={(selectedProject, selectedIssue) =>
                        onOpen(selectedProject, repository, selectedIssue)
                      }
                      profiles={profiles}
                      project={project}
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
