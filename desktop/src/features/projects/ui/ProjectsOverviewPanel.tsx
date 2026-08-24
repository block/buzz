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
import * as React from "react";

import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type {
  Project,
  ProjectActivitySummary,
  ProjectIssue,
  ProjectPullRequest,
} from "@/features/projects/hooks";
import {
  type ProjectSelectionItem,
  projectSelectionPresentation,
} from "@/features/projects/lib/projectSelection";
import type { ProjectsFilter } from "@/features/projects/lib/projectsViewHelpers";
import type { ProjectsActivityDigest } from "@/features/projects/lib/projectsActivityDigest";
import { useProjectSelection } from "@/features/projects/lib/useProjectSelection";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { ProjectsCreateMenu } from "./ProjectsCreateMenu";
import { ProjectsOverviewPeople } from "./ProjectsOverviewRail";
import { ProjectsSelectionCountMenu } from "./ProjectsSelectionCountMenu";
import {
  type OverviewContextStatIcon,
  type ProjectsOverviewSection,
  projectsOverviewContext,
} from "./projectsOverviewContext";

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
  onChatWithAgent: (items: ProjectSelectionItem[]) => void;
  onCreateIssue: () => void;
  onCreateProject: () => void;
  onCreatePullRequest: () => void;
  onSelectSection: (section: ProjectsOverviewSection) => void;
  profiles?: UserProfileLookup;
  projectReadModels: Project[];
  projects: Project[];
  pullRequests: ProjectPullRequest[];
  repositorySummaries?: Record<string, ProjectActivitySummary>;
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
      className="-mx-2 h-7 w-[calc(100%+1rem)] justify-start gap-3 rounded-md px-2 text-left text-sm font-normal text-muted-foreground hover:bg-muted/70 hover:text-foreground [&_svg]:h-4 [&_svg]:w-4 [&_svg]:shrink-0 [&_svg]:text-muted-foreground"
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
      <span className="font-medium tabular-nums text-muted-foreground/65">
        {count}
      </span>
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

export function ProjectsActivityIntro({
  digest,
}: {
  digest: ProjectsActivityDigest;
}) {
  return (
    <section
      className="pb-6 pt-8 text-left"
      data-testid="projects-activity-intro"
    >
      <h2
        className="text-xl font-semibold tracking-tight text-foreground"
        data-testid="projects-page-header"
      >
        Projects Activity
      </h2>
      <p
        className="mt-2 max-w-2xl text-sm text-muted-foreground"
        data-testid="projects-activity-summary"
      >
        {digest.prefix}{" "}
        {digest.highlights.map((highlight, index) => (
          <span key={highlight}>
            {index > 0
              ? index === digest.highlights.length - 1
                ? ", and "
                : ", "
              : null}
            <strong className="font-medium text-foreground/90">
              {highlight}
            </strong>
          </span>
        ))}
        {digest.suffix}
      </p>
    </section>
  );
}

export function ProjectsOverviewContextPanel({
  filter,
  issues,
  onChatWithAgent,
  onCreateIssue,
  onCreateProject,
  onCreatePullRequest,
  onSelectSection,
  profiles,
  projectReadModels,
  projects,
  pullRequests,
  repositorySummaries,
  summaries,
}: ProjectsOverviewContextPanelProps) {
  const selection = useProjectSelection();
  const selectionItems = selection?.items;
  const selectionPresentation = React.useMemo(
    () => projectSelectionPresentation(selectionItems ?? []),
    [selectionItems],
  );
  // Memoized: the rail re-renders with every Projects-view state change, and
  // the context stats walk every issue and pull request in the community.
  const context = React.useMemo(
    () =>
      projectsOverviewContext({
        filter,
        issues,
        projectReadModels,
        projects,
        pullRequests,
        repositorySummaries,
        summaries,
      }),
    [
      filter,
      issues,
      projectReadModels,
      projects,
      pullRequests,
      repositorySummaries,
      summaries,
    ],
  );
  const actionHandler =
    context.action?.kind === "issue"
      ? onCreateIssue
      : context.action?.kind === "pullRequest"
        ? onCreatePullRequest
        : onCreateProject;

  return (
    <div
      className={cn(
        "min-w-0 overflow-hidden",
        selectionPresentation && "rounded-xl bg-background/90",
      )}
      data-testid="projects-overview-context-panel"
    >
      <div className="px-4 pb-3 pt-3">
        {selectionPresentation && selection ? (
          <ProjectsSelectionCountMenu
            onChatWithAgent={onChatWithAgent}
            onCreatePullRequest={onCreatePullRequest}
            presentation={selectionPresentation}
            selectionItems={selection.items}
          />
        ) : (
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
        )}
        {selectionPresentation ? null : (
          <div className="space-y-2.5 pt-2">
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
              className="space-y-2.5 text-sm"
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
            {context.people.length > 0 ? (
              <div className="mt-3 space-y-3 py-3">
                <ProjectsOverviewPeople
                  people={context.people}
                  profiles={profiles}
                />
              </div>
            ) : null}
          </div>
        )}
      </div>
    </div>
  );
}
