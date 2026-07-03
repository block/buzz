import { CircleDot, FolderGit2, GitPullRequest, Radio } from "lucide-react";
import type * as React from "react";

import { WorkspaceEmojiIcon } from "@/features/workspaces/ui/WorkspaceSwitcher";
import type {
  Project,
  ProjectActivitySummary,
} from "@/features/projects/hooks";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { OverviewRailSection } from "./ProjectOverviewPanel";
import { ProjectsContributionGraph } from "./ProjectsContributionGraph";

export type ProjectsOverviewSection =
  | "repositories"
  | "prs"
  | "local"
  | "issues";

type ProjectsOverviewPanelProps = {
  localRepositoryCount: number;
  onSelectSection: (section: ProjectsOverviewSection) => void;
  profiles?: UserProfileLookup;
  projects: Project[];
  relayName: string;
  summaries?: Record<string, ProjectActivitySummary>;
};

function unitLabel(count: number, singular: string, plural = `${singular}s`) {
  return count === 1 ? singular : plural;
}

function projectPeople(
  project: Project,
  summary: ProjectActivitySummary | undefined,
) {
  return [
    ...new Set(
      [
        project.owner,
        ...project.contributors,
        ...(summary?.participantPubkeys ?? []),
      ].map(normalizePubkey),
    ),
  ];
}

function overviewPeople(
  projects: Project[],
  summaries: Record<string, ProjectActivitySummary> | undefined,
) {
  return [
    ...new Set(
      projects.flatMap((project) =>
        projectPeople(project, summaries?.[project.repoAddress]),
      ),
    ),
  ];
}

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
  unit,
}: {
  count: number;
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  onClick: () => void;
  unit: string;
}) {
  return (
    <button
      className="flex flex-col rounded-xl bg-card px-3.5 py-3 text-left shadow-xs transition-colors hover:bg-muted/40"
      onClick={onClick}
      type="button"
    >
      <span className="flex w-full items-center justify-between gap-2">
        <span className="text-xs font-medium text-muted-foreground">
          {label}
        </span>
        <Icon className="h-3.5 w-3.5 text-muted-foreground/70" />
      </span>
      <span className="mt-2 flex min-w-0 items-baseline gap-1.5">
        <span className="text-2xl font-semibold leading-none tracking-tight text-foreground">
          {count}
        </span>
        <span className="truncate text-xs text-muted-foreground">{unit}</span>
      </span>
    </button>
  );
}

export function ProjectsOverviewPanel({
  localRepositoryCount,
  onSelectSection,
  profiles,
  projects,
  relayName,
  summaries,
}: ProjectsOverviewPanelProps) {
  const stats = overviewStats(projects, summaries);
  const people = overviewPeople(projects, summaries);
  const activityByDay = overviewActivityByDay(projects, summaries);

  return (
    <section className="mb-4 grid gap-4 xl:grid-cols-[minmax(0,1fr)_18rem]">
      <div className="min-w-0 space-y-4">
        <div className="overflow-hidden rounded-2xl border border-border/50 bg-muted/20 p-4">
          <div className="flex min-w-0 items-start gap-3">
            <WorkspaceEmojiIcon className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-muted/60 text-2xl" />
            <div className="-mt-1 min-w-0 flex-1 space-y-0.5">
              <h2 className="text-xl font-semibold leading-7 tracking-tight text-foreground">
                {relayName} Projects
              </h2>
              <p className="max-w-2xl text-sm font-normal text-muted-foreground">
                Browse shared repositories, pull requests, and local project
                checkouts in this workspace.
              </p>
            </div>
          </div>
          <div className="mt-4 grid gap-2 sm:grid-cols-2 xl:grid-cols-4">
            <StatPill
              count={projects.length}
              icon={FolderGit2}
              label="Repositories"
              onClick={() => onSelectSection("repositories")}
              unit={unitLabel(projects.length, "project")}
            />
            <StatPill
              count={stats.prs}
              icon={GitPullRequest}
              label="Pull requests"
              onClick={() => onSelectSection("prs")}
              unit={unitLabel(stats.prs, "PR")}
            />
            <StatPill
              count={localRepositoryCount}
              icon={Radio}
              label="Local"
              onClick={() => onSelectSection("local")}
              unit={unitLabel(localRepositoryCount, "checkout")}
            />
            <StatPill
              count={stats.issues}
              icon={CircleDot}
              label="Issues"
              onClick={() => onSelectSection("issues")}
              unit={unitLabel(stats.issues, "issue")}
            />
          </div>
        </div>
        <div className="overflow-hidden rounded-2xl border border-border/50 bg-muted/20 p-4">
          <div className="flex items-center justify-between gap-2">
            <h3 className="text-sm font-semibold text-foreground">
              Contribution activity
            </h3>
            <span className="text-xs text-muted-foreground">
              Commits, PRs & issues across all projects
            </span>
          </div>
          <ProjectsContributionGraph
            activityByDay={activityByDay}
            className="mt-3"
          />
        </div>
      </div>
      <aside className="space-y-4 rounded-xl border border-border/50 bg-card/60 p-4">
        <OverviewRailSection title="People">
          <div className="flex flex-wrap gap-1.5">
            {people.slice(0, 18).map((pubkey) => {
              const profile = profiles?.[normalizePubkey(pubkey)];
              const label = resolveUserLabel({ pubkey, profiles });
              return (
                <Tooltip key={pubkey}>
                  <TooltipTrigger asChild>
                    <span className="inline-flex">
                      <UserAvatar
                        accent={profile?.isAgent === true}
                        avatarUrl={profile?.avatarUrl ?? null}
                        displayName={label}
                        size="sm"
                      />
                    </span>
                  </TooltipTrigger>
                  <TooltipContent>{label}</TooltipContent>
                </Tooltip>
              );
            })}
          </div>
        </OverviewRailSection>
      </aside>
    </section>
  );
}
