import { CircleDot, FolderGit2, GitPullRequest, Radio } from "lucide-react";
import type * as React from "react";

import { WorkspaceEmojiIcon } from "@/features/workspaces/ui/WorkspaceSwitcher";
import type {
  Project,
  ProjectActivitySummary,
} from "@/features/projects/hooks";
import {
  ProjectsContributionGraph,
  ProjectsContributionLegend,
} from "./ProjectsContributionGraph";

export type ProjectsOverviewSection =
  | "repositories"
  | "prs"
  | "local"
  | "issues";

type ProjectsOverviewPanelProps = {
  children: React.ReactNode;
  localRepositoryCount: number;
  metadata: React.ReactNode;
  onSelectSection: (section: ProjectsOverviewSection) => void;
  projects: Project[];
  relayName: string;
  summaries?: Record<string, ProjectActivitySummary>;
};

function overviewStats(
  projects: Project[],
  summaries: Record<string, ProjectActivitySummary> | undefined,
) {
  return projects.reduce(
    (stats, project) => {
      const summary = summaries?.[project.repoAddress];
      return {
        issues: stats.issues + (summary?.issueCount ?? 0),
        prs: stats.prs + (summary?.prCount ?? 0),
      };
    },
    { issues: 0, prs: 0 },
  );
}

function overviewActivityByDay(
  projects: Project[],
  summaries: Record<string, ProjectActivitySummary> | undefined,
) {
  const merged: Record<string, number> = {};
  for (const project of projects) {
    const byDay = summaries?.[project.repoAddress]?.activityByDay;
    if (!byDay) continue;
    for (const [day, count] of Object.entries(byDay)) {
      merged[day] = (merged[day] ?? 0) + count;
    }
  }
  return merged;
}

function StatPill({
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
      className="flex flex-col rounded-lg border border-border/60 bg-card px-3.5 py-3 text-left transition-colors hover:bg-muted/30"
      onClick={onClick}
      type="button"
    >
      <span className="flex w-full items-center justify-between gap-2">
        <span className="text-xs font-medium text-muted-foreground">
          {label}
        </span>
        <Icon className="h-3.5 w-3.5 text-muted-foreground/70" />
      </span>
      <span className="mt-2 text-2xl font-semibold leading-none tracking-tight text-foreground">
        {count}
      </span>
    </button>
  );
}

export function ProjectsOverviewPanel({
  children,
  localRepositoryCount,
  metadata,
  onSelectSection,
  projects,
  relayName,
  summaries,
}: ProjectsOverviewPanelProps) {
  const stats = overviewStats(projects, summaries);
  const activityByDay = overviewActivityByDay(projects, summaries);

  return (
    <section className="-mx-4 mb-4 bg-card">
      <div className="grid xl:grid-cols-[minmax(0,1fr)_18rem] 2xl:grid-cols-[minmax(0,1fr)_24rem]">
        <div className="order-1 flex min-w-0 items-center gap-3 p-4 xl:order-none">
          <WorkspaceEmojiIcon className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl border border-border/60 bg-muted/40 text-3xl" />
          <div className="min-w-0 flex-1 space-y-1">
            <h2 className="text-xl font-semibold leading-6 tracking-tight text-foreground">
              {relayName} Projects
            </h2>
            <p className="line-clamp-2 max-w-2xl text-sm font-normal text-muted-foreground sm:line-clamp-none">
              Browse shared repositories, pull requests, and local project
              checkouts in this workspace.
            </p>
          </div>
        </div>
        <div className="hidden xl:block" />
        <div className="order-2 grid grid-cols-2 gap-2 p-4 sm:gap-3 xl:order-none xl:col-start-1 xl:row-start-2 xl:grid-cols-4">
          <StatPill
            count={projects.length}
            icon={FolderGit2}
            label="Repositories"
            onClick={() => onSelectSection("repositories")}
          />
          <StatPill
            count={stats.prs}
            icon={GitPullRequest}
            label="Pull requests"
            onClick={() => onSelectSection("prs")}
          />
          <StatPill
            count={localRepositoryCount}
            icon={Radio}
            label="Local"
            onClick={() => onSelectSection("local")}
          />
          <StatPill
            count={stats.issues}
            icon={CircleDot}
            label="Issues"
            onClick={() => onSelectSection("issues")}
          />
        </div>
        <div className="order-3 min-w-0 overflow-hidden xl:order-none xl:col-start-1 xl:row-start-3">
          <div className="flex items-center justify-between gap-3 px-4 pt-3">
            <h3 className="text-sm font-semibold text-foreground">
              Contribution Activity
            </h3>
            <ProjectsContributionLegend />
          </div>
          <div className="overflow-x-auto xl:overflow-visible">
            <ProjectsContributionGraph
              activityByDay={activityByDay}
              className="min-w-[32rem] p-4"
            />
          </div>
        </div>
        <aside className="order-5 min-w-0 border-t border-border/40 px-4 py-4 xl:order-none xl:col-start-2 xl:row-span-2 xl:row-start-2 xl:border-t-0">
          {metadata}
        </aside>
        <div className="order-4 min-w-0 p-4 pt-2 xl:order-none xl:col-start-1 xl:row-start-4">
          {children}
        </div>
      </div>
    </section>
  );
}
