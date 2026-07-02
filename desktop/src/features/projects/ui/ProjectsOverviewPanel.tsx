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

type ProjectsOverviewPanelProps = {
  localRepositoryCount: number;
  profiles?: UserProfileLookup;
  projects: Project[];
  relayName: string;
  summaries?: Record<string, ProjectActivitySummary>;
};

function pluralize(count: number, singular: string, plural = `${singular}s`) {
  return `${count} ${count === 1 ? singular : plural}`;
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
        events: stats.events + (summary?.activityCount ?? 0),
        issues: stats.issues + (summary?.issueCount ?? 0),
        prs: stats.prs + (summary?.prCount ?? 0),
      };
    },
    { events: 0, issues: 0, prs: 0 },
  );
}

function StatPill({
  icon: Icon,
  label,
  value,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  value: string;
}) {
  return (
    <div className="rounded-xl border border-border/50 bg-background/55 px-3 py-2">
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <Icon className="h-3.5 w-3.5" />
        {label}
      </div>
      <p className="mt-1 text-sm font-semibold text-foreground">{value}</p>
    </div>
  );
}

export function ProjectsOverviewPanel({
  localRepositoryCount,
  profiles,
  projects,
  relayName,
  summaries,
}: ProjectsOverviewPanelProps) {
  const stats = overviewStats(projects, summaries);
  const people = overviewPeople(projects, summaries);

  return (
    <section className="mb-4">
      <div className="rounded-xl border border-border/60 bg-card/60 p-4">
        <div className="flex min-w-0 items-start gap-3">
          <WorkspaceEmojiIcon className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-border/70 bg-muted/60 text-2xl shadow-inner" />
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
            icon={FolderGit2}
            label="Repositories"
            value={pluralize(projects.length, "project")}
          />
          <StatPill
            icon={GitPullRequest}
            label="Pull requests"
            value={pluralize(stats.prs, "PR")}
          />
          <StatPill
            icon={Radio}
            label="Local"
            value={pluralize(localRepositoryCount, "checkout")}
          />
          <StatPill
            icon={CircleDot}
            label="Issues"
            value={pluralize(stats.issues, "issue")}
          />
        </div>
        <div className="mt-4 flex flex-wrap gap-1.5">
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
      </div>
    </section>
  );
}
