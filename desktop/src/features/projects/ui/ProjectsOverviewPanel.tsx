import {
  CircleDot,
  FileCode2,
  FolderGit2,
  GitCommitHorizontal,
  GitPullRequest,
  Radio,
  Users,
} from "lucide-react";
import type * as React from "react";

import { WorkspaceEmojiIcon } from "@/features/workspaces/ui/WorkspaceSwitcher";
import type {
  Project,
  ProjectActivitySummary,
  ProjectRepoSnapshot,
} from "@/features/projects/hooks";
import {
  languageForPath,
  topLanguagesFromCounts,
} from "@/features/projects/lib/projectLanguages";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { LanguageChips, OverviewRailSection } from "./ProjectOverviewPanel";
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
  /** Repo snapshots keyed by project ID, for workspace-wide aggregates. */
  snapshots?: Record<string, ProjectRepoSnapshot>;
  snapshotsLoading?: boolean;
  summaries?: Record<string, ProjectActivitySummary>;
};

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

function overviewLanguages(
  snapshots: Record<string, ProjectRepoSnapshot> | undefined,
) {
  const counts: Record<string, number> = {};
  for (const snapshot of Object.values(snapshots ?? {})) {
    for (const file of snapshot.files) {
      const language = languageForPath(file.path);
      if (language) counts[language] = (counts[language] ?? 0) + 1;
    }
  }
  return topLanguagesFromCounts(counts);
}

function overviewRepoTotals(
  snapshots: Record<string, ProjectRepoSnapshot> | undefined,
) {
  const contributorEmails = new Set<string>();
  let files = 0;
  let latestCommit: ProjectRepoSnapshot["latestCommit"] = null;

  for (const snapshot of Object.values(snapshots ?? {})) {
    files += snapshot.files.length;
    for (const contributor of snapshot.contributors) {
      contributorEmails.add(
        contributor.email.toLowerCase() || contributor.name.toLowerCase(),
      );
    }
    if (
      snapshot.latestCommit &&
      snapshot.latestCommit.timestamp > (latestCommit?.timestamp ?? 0)
    ) {
      latestCommit = snapshot.latestCommit;
    }
  }

  return { contributors: contributorEmails.size, files, latestCommit };
}

function RepoTotalRow({
  icon: Icon,
  label,
  value,
  mono,
}: {
  icon?: React.ComponentType<{ className?: string }>;
  label: string;
  value: React.ReactNode;
  mono?: boolean;
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <dt className="flex items-center gap-1.5 text-muted-foreground">
        {Icon ? <Icon className="h-3.5 w-3.5" /> : null}
        {label}
      </dt>
      <dd
        className={
          mono
            ? "font-mono text-xs text-foreground"
            : "font-medium text-foreground"
        }
      >
        {value}
      </dd>
    </div>
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
  localRepositoryCount,
  onSelectSection,
  profiles,
  projects,
  relayName,
  snapshots,
  snapshotsLoading,
  summaries,
}: ProjectsOverviewPanelProps) {
  const stats = overviewStats(projects, summaries);
  const people = overviewPeople(projects, summaries);
  const activityByDay = overviewActivityByDay(projects, summaries);
  const languages = overviewLanguages(snapshots);
  const repoTotals = overviewRepoTotals(snapshots);
  const scanning = Boolean(snapshotsLoading);

  return (
    <section className="-mx-4 mb-4 bg-card">
      <div className="flex min-w-0 items-start gap-3 p-4">
        <WorkspaceEmojiIcon className="flex h-10 w-10 shrink-0 items-center justify-center border border-border/60 bg-muted/40 text-2xl" />
        <div className="-mt-1 min-w-0 flex-1">
          <h2 className="text-xl font-semibold leading-6 tracking-tight text-foreground">
            {relayName} Projects
          </h2>
          <p className="max-w-2xl text-sm font-normal text-muted-foreground">
            Browse shared repositories, pull requests, and local project
            checkouts in this workspace.
          </p>
        </div>
      </div>
      <div className="grid xl:grid-cols-[minmax(0,1fr)_18rem]">
        <div className="min-w-0">
          <div className="grid gap-3 p-4 sm:grid-cols-2 xl:grid-cols-4">
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
          <div className="overflow-hidden">
            <h3 className="px-4 pt-3 text-sm font-semibold text-foreground">
              Contribution Activity
            </h3>
            <ProjectsContributionGraph
              activityByDay={activityByDay}
              className="p-4"
            />
          </div>
        </div>
        <aside className="[&>section]:p-4">
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
          <OverviewRailSection title="Top Languages">
            {languages.length > 0 ? (
              <LanguageChips languages={languages} />
            ) : (
              <p className="text-sm text-muted-foreground">
                {scanning
                  ? "Scanning repositories..."
                  : "No language data is available yet."}
              </p>
            )}
          </OverviewRailSection>
          <OverviewRailSection title="Repositories">
            <dl className="space-y-2 text-sm">
              <RepoTotalRow
                icon={FolderGit2}
                label="Repositories"
                value={projects.length}
              />
              <RepoTotalRow
                icon={GitCommitHorizontal}
                label="Latest"
                mono
                value={
                  repoTotals.latestCommit
                    ? repoTotals.latestCommit.shortHash
                    : scanning
                      ? "..."
                      : "None"
                }
              />
              <RepoTotalRow
                icon={FileCode2}
                label="Files"
                value={
                  scanning && repoTotals.files === 0 ? "..." : repoTotals.files
                }
              />
              <RepoTotalRow
                icon={Users}
                label="Contributors"
                value={
                  scanning && repoTotals.contributors === 0
                    ? "..."
                    : repoTotals.contributors
                }
              />
              <RepoTotalRow
                icon={GitPullRequest}
                label="Pull Requests"
                value={stats.prs}
              />
            </dl>
          </OverviewRailSection>
        </aside>
      </div>
    </section>
  );
}
