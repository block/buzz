import { MessageSquare } from "lucide-react";
import type * as React from "react";

import type { ProjectIssue } from "@/features/projects/hooks";
import { relativeTime } from "@/features/projects/lib/projectsViewHelpers";
import { projectTaskCategoryLabel } from "@/features/projects/projectTaskCategories";
import { cn } from "@/shared/lib/cn";

type KanbanIssueItem = {
  issue: ProjectIssue;
  onOpen: () => void;
};

type KanbanIssueGroup = {
  icon: React.ReactNode;
  items: KanbanIssueItem[];
  status: ProjectIssue["status"];
};

function ProjectIssueKanbanCard({ issue, onOpen }: KanbanIssueItem) {
  return (
    <button
      className="group w-full rounded-lg border border-border/70 bg-background/65 p-3 text-left shadow-xs transition-colors hover:border-border hover:bg-muted/35 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
      data-project-event-id={issue.id}
      data-testid="project-issue-kanban-card"
      onClick={onOpen}
      type="button"
    >
      <span className="line-clamp-2 text-sm font-medium leading-5 text-foreground group-hover:text-primary">
        {issue.title}
      </span>
      {issue.content ? (
        <span className="mt-1.5 line-clamp-2 text-xs leading-4 text-muted-foreground/75">
          {issue.content}
        </span>
      ) : null}
      <span className="mt-3 flex items-center gap-2 text-2xs text-muted-foreground/65">
        <span className="font-medium tabular-nums">
          #{issue.id.slice(0, 8)}
        </span>
        <span aria-hidden="true">/</span>
        <span className="truncate">
          {projectTaskCategoryLabel(issue.category)}
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-1">
          <MessageSquare aria-hidden="true" className="h-3 w-3" />
          {issue.comments.length}
        </span>
      </span>
      <span
        className="mt-2 block text-2xs text-muted-foreground/50"
        title={new Date(issue.updatedAt * 1_000).toLocaleString()}
      >
        Updated {relativeTime(issue.updatedAt)}
      </span>
    </button>
  );
}

/** Status-column view of a project's tasks. Cards open the existing task detail. */
export function ProjectIssueKanbanBoard({
  groups,
}: {
  groups: KanbanIssueGroup[];
}) {
  return (
    <section
      aria-label="Project task board"
      className="grid auto-cols-[minmax(17rem,1fr)] grid-flow-col gap-3 overflow-x-auto px-3 pb-4 pt-2"
      data-testid="project-issue-kanban-board"
    >
      {groups.map(({ icon, items, status }) => (
        <section
          aria-labelledby={`project-kanban-${status.replaceAll(" ", "-").toLowerCase()}`}
          className="min-h-48 rounded-xl border border-border/60 bg-muted/15"
          data-testid="project-issue-kanban-column"
          key={status}
        >
          <header className="sticky top-0 z-10 flex items-center gap-2 rounded-t-xl border-b border-border/50 bg-background/90 px-3 py-2.5 backdrop-blur">
            {icon}
            <h3
              className="text-xs font-semibold text-foreground"
              id={`project-kanban-${status.replaceAll(" ", "-").toLowerCase()}`}
            >
              {status}
            </h3>
            <span
              className={cn(
                "ml-auto min-w-5 rounded-full bg-muted px-1.5 py-0.5 text-center text-2xs font-medium tabular-nums text-muted-foreground",
                items.length === 0 && "opacity-60",
              )}
            >
              {items.length}
            </span>
          </header>
          <div className="space-y-2 p-2">
            {items.length > 0 ? (
              items.map((item) => (
                <ProjectIssueKanbanCard key={item.issue.id} {...item} />
              ))
            ) : (
              <p className="px-2 py-5 text-center text-xs text-muted-foreground/55">
                No tasks
              </p>
            )}
          </div>
        </section>
      ))}
    </section>
  );
}
