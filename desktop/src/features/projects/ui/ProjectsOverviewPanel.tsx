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
        issues: stats.issues + (summary?.issueCount ?? 0),
        prs: stats.prs + (summary?.prCount ?? 0),
      };
    },
    { issues: 0, prs: 0 },
  );
}

function StatPill({
  icon: Icon,
  label,
  onClick,
  value,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  onClick: () => void;
  value: string;
}) {
  return (
    <button
      className="flex items-center gap-3 rounded-xl bg-card px-3 py-2.5 text-left shadow-xs transition-colors hover:bg-muted/40"
      onClick={onClick}
      type="button"
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60">
        <Icon className="h-4 w-4 text-muted-foreground" />
      </span>
      <div className="min-w-0">
        <span className="block text-xs font-medium text-foreground">
          {label}
        </span>
        <span className="mt-0.5 block truncate text-sm text-muted-foreground">
          {value}
        </span>
      </div>
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

  return (
    <section className="mb-4 grid gap-4 xl:grid-cols-[minmax(0,1fr)_18rem]">
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
            icon={FolderGit2}
            label="Repositories"
            onClick={() => onSelectSection("repositories")}
            value={pluralize(projects.length, "project")}
          />
          <StatPill
            icon={GitPullRequest}
            label="Pull requests"
            onClick={() => onSelectSection("prs")}
            value={pluralize(stats.prs, "PR")}
          />
          <StatPill
            icon={Radio}
            label="Local"
            onClick={() => onSelectSection("local")}
            value={pluralize(localRepositoryCount, "checkout")}
          />
          <StatPill
            icon={CircleDot}
            label="Issues"
            onClick={() => onSelectSection("issues")}
            value={pluralize(stats.issues, "issue")}
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
