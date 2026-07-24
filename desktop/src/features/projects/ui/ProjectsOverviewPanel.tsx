import { CircleDot, FolderGit2, GitPullRequest, Radio } from "lucide-react";
import type * as React from "react";

import type {
  Project,
  ProjectActivitySummary,
} from "@/features/projects/hooks";

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
      <span className="mt-auto pt-4 text-4xl font-semibold leading-none tracking-tight text-foreground">
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
  summaries,
}: ProjectsOverviewPanelProps) {
  const stats = overviewStats(projects, summaries);

  return (
    <section className="-mx-4 mb-4 bg-card">
      <div className="grid xl:grid-cols-[minmax(0,1fr)_18rem] 2xl:grid-cols-[minmax(0,1fr)_24rem]">
        <div className="order-1 grid grid-cols-2 gap-2 p-4 pt-0 sm:gap-3 xl:order-none xl:col-start-1 xl:row-start-1 xl:grid-cols-4">
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
        {metadata}
        <div className="order-2 min-w-0 p-4 pt-2 xl:order-none xl:col-start-1 xl:row-start-2">
          {children}
        </div>
      </div>
    </section>
  );
}
