import {
  CheckCircle2,
  CircleDot,
  FolderGit2,
  Folders,
  GitMerge,
  GitPullRequest,
  Hash,
  Plus,
} from "lucide-react";
import type * as React from "react";

import type {
  Project,
  ProjectActivitySummary,
  ProjectIssue,
  ProjectPullRequest,
} from "@/features/projects/hooks";
import type { ProjectsFilter } from "@/features/projects/lib/projectsViewHelpers";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { Button } from "@/shared/ui/button";
import { ProjectsCreateMenu } from "./ProjectsCreateMenu";
import {
  type OverviewContextStatIcon,
  type ProjectsOverviewSection,
  projectsOverviewContext,
} from "./projectsOverviewContext";
import {
  ProjectsOverviewActivityGraph,
  ProjectsOverviewPeople,
} from "./ProjectsOverviewRail";

export type { ProjectsOverviewSection };

const STAT_ICONS: Record<
  OverviewContextStatIcon,
  React.ComponentType<{ className?: string }>
> = {
  channels: Hash,
  completed: CheckCircle2,
  merged: GitMerge,
  projects: Folders,
  repositories: FolderGit2,
  reviews: GitPullRequest,
  tasks: CircleDot,
};

type ProjectsOverviewPanelProps = {
  children: React.ReactNode;
};

type ProjectsOverviewContextPanelProps = {
  filter: ProjectsFilter;
  issues: ProjectIssue[];
  onCreateIssue: () => void;
  onCreateProject: () => void;
  onCreatePullRequest: () => void;
  onSelectSection: (section: ProjectsOverviewSection) => void;
  profiles?: UserProfileLookup;
  projects: Project[];
  pullRequests: ProjectPullRequest[];
  summaries?: Record<string, ProjectActivitySummary>;
};

function OverviewActionButton({
  children,
  onClick,
  testId,
}: {
  children: React.ReactNode;
  onClick: () => void;
  testId?: string;
}) {
  return (
    <Button
      className="-mx-2 h-7 w-[calc(100%+1rem)] justify-start gap-3 rounded-md px-2 text-left text-sm font-normal hover:bg-muted/70 [&_svg]:h-4 [&_svg]:w-4 [&_svg]:shrink-0 [&_svg]:text-muted-foreground"
      data-testid={testId}
      onClick={onClick}
      size="sm"
      type="button"
      variant="ghost"
    >
      {children}
    </Button>
  );
}

function OverviewStatRow({
  count,
  icon: Icon,
  label,
  onClick,
}: {
  count: number;
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      className="-mx-2 flex h-7 w-[calc(100%+1rem)] items-center justify-between gap-3 rounded-md px-2 text-left text-sm hover:bg-muted/70 [&_svg]:h-4 [&_svg]:w-4 [&_svg]:shrink-0 [&_svg]:text-muted-foreground"
      data-testid="projects-overview-stat"
      onClick={onClick}
      type="button"
    >
      <span className="flex min-w-0 items-center gap-3 text-muted-foreground">
        <Icon className="h-3.5 w-3.5 shrink-0" />
        {label}
      </span>
      <span className="font-medium tabular-nums text-foreground">{count}</span>
    </button>
  );
}

export function ProjectsOverviewPanel({
  children,
}: ProjectsOverviewPanelProps) {
  return (
    <section data-testid="projects-overview-panel">
      <div className="min-w-0">{children}</div>
    </section>
  );
}

export function ProjectsActivityIntro() {
  return (
    <section
      className="pb-8 pt-5 text-center"
      data-testid="projects-activity-intro"
    >
      <h2
        className="text-xl font-semibold tracking-tight text-foreground"
        data-testid="projects-page-header"
      >
        Welcome to Activity
      </h2>
      <p className="mx-auto mt-2 max-w-md text-sm text-muted-foreground">
        Keep up with commits, reviews, and tasks across your projects.
      </p>
    </section>
  );
}

export function ProjectsOverviewContextPanel({
  filter,
  issues,
  onCreateIssue,
  onCreateProject,
  onCreatePullRequest,
  onSelectSection,
  profiles,
  projects,
  pullRequests,
  summaries,
}: ProjectsOverviewContextPanelProps) {
  const context = projectsOverviewContext({
    filter,
    issues,
    projects,
    pullRequests,
    summaries,
  });
  const actionHandler =
    context.action?.kind === "issue"
      ? onCreateIssue
      : context.action?.kind === "pullRequest"
        ? onCreatePullRequest
        : onCreateProject;

  return (
    <div
      className="min-w-0 overflow-hidden rounded-2xl bg-background"
      data-testid="projects-overview-context-panel"
    >
      <div className="px-4 pb-3 pt-3">
        <div className="flex min-w-0 items-center justify-between gap-2">
          <h2
            className="min-w-0 truncate text-sm font-normal text-muted-foreground/70"
            data-testid="projects-overview-context-title"
          >
            {context.title}
          </h2>
          <ProjectsCreateMenu
            compact
            onCreateIssue={onCreateIssue}
            onCreateProject={onCreateProject}
            onCreatePullRequest={onCreatePullRequest}
          />
        </div>
        <div className="space-y-0.5 pt-2">
          {context.action ? (
            <OverviewActionButton
              onClick={actionHandler}
              testId={context.action.testId}
            >
              <Plus className="h-3.5 w-3.5" />
              {context.action.label}
            </OverviewActionButton>
          ) : null}
          <section
            className="text-sm"
            data-testid="projects-overview-stats-pod"
          >
            {context.stats.map((stat) => (
              <OverviewStatRow
                count={stat.count}
                icon={STAT_ICONS[stat.icon]}
                key={`${stat.section}:${stat.label}`}
                label={stat.label}
                onClick={() => onSelectSection(stat.section)}
              />
            ))}
          </section>
        </div>
        {context.people.length > 0 || context.activityByDay ? (
          <div className="mt-3 space-y-3 border-border/50 border-t py-3">
            <ProjectsOverviewPeople
              people={context.people}
              profiles={profiles}
            />
            {context.activityByDay ? (
              <div data-testid="projects-overview-activity">
                <ProjectsOverviewActivityGraph
                  activityByDay={context.activityByDay}
                />
              </div>
            ) : null}
          </div>
        ) : null}
      </div>
    </div>
  );
}
