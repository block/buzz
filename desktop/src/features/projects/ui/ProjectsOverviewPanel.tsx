import { Activity, FolderGit2, GitPullRequest, Radio } from "lucide-react";
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
import { UserAvatar } from "@/shared/ui/UserAvatar";

type ProjectsOverviewPanelProps = {
  localRepositoryCount: number;
  onOpenProject: (project: Project) => void;
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

function recentProjects(
  projects: Project[],
  summaries: Record<string, ProjectActivitySummary> | undefined,
) {
  return [...projects]
    .sort((left, right) => {
      const leftUpdated =
        summaries?.[left.repoAddress]?.updatedAt ?? left.createdAt;
      const rightUpdated =
        summaries?.[right.repoAddress]?.updatedAt ?? right.createdAt;
      return rightUpdated - leftUpdated;
    })
    .slice(0, 3);
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
  onOpenProject,
  profiles,
  projects,
  relayName,
  summaries,
}: ProjectsOverviewPanelProps) {
  const stats = overviewStats(projects, summaries);
  const people = overviewPeople(projects, summaries);
  const recent = recentProjects(projects, summaries);

  return (
    <section className="mb-4 grid gap-4 xl:grid-cols-[minmax(0,1fr)_18rem]">
      <div className="rounded-xl border border-border/60 bg-card/60 p-4">
        <div className="flex min-w-0 items-start gap-3">
          <WorkspaceEmojiIcon className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl border border-border/70 bg-muted/60 text-3xl shadow-inner" />
          <div className="min-w-0 flex-1">
            <h2 className="text-lg font-semibold text-foreground">
              {relayName} Projects
            </h2>
            <p className="mt-1 max-w-2xl text-sm text-muted-foreground">
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
            icon={Activity}
            label="Activity"
            value={pluralize(stats.events, "event")}
          />
        </div>
        {recent.length > 0 ? (
          <div className="mt-4 border-border/50 border-t pt-3">
            <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
              Recent Projects
            </h3>
            <div className="mt-2 grid gap-2 md:grid-cols-3">
              {recent.map((project) => (
                <button
                  className="min-w-0 rounded-lg border border-border/50 bg-background/45 p-3 text-left transition-colors hover:bg-muted/35 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                  key={project.id}
                  onClick={() => onOpenProject(project)}
                  type="button"
                >
                  <p className="truncate text-sm font-semibold text-foreground">
                    {project.name}
                  </p>
                  <p className="mt-1 line-clamp-2 text-xs text-muted-foreground">
                    {project.description || "A shared project repository."}
                  </p>
                </button>
              ))}
            </div>
          </div>
        ) : null}
      </div>
      <aside className="space-y-3 rounded-xl border border-border/60 bg-card/60 p-4">
        <div>
          <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            People
          </h3>
          <div className="mt-2 flex flex-wrap gap-1.5">
            {people.slice(0, 18).map((pubkey) => {
              const profile = profiles?.[normalizePubkey(pubkey)];
              return (
                <UserAvatar
                  accent={profile?.isAgent === true}
                  avatarUrl={profile?.avatarUrl ?? null}
                  displayName={resolveUserLabel({ pubkey, profiles })}
                  key={pubkey}
                  size="sm"
                />
              );
            })}
          </div>
          <p className="mt-2 text-xs text-muted-foreground">
            {pluralize(people.length, "person", "people")} across active
            projects.
          </p>
        </div>
        <div className="border-border/50 border-t pt-3">
          <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Status
          </h3>
          <div className="mt-2 space-y-1.5 text-sm text-muted-foreground">
            <div className="flex items-center justify-between gap-3">
              <span>Issues</span>
              <span className="font-medium text-foreground">
                {stats.issues}
              </span>
            </div>
            <div className="flex items-center justify-between gap-3">
              <span>Pull requests</span>
              <span className="font-medium text-foreground">{stats.prs}</span>
            </div>
            <div className="flex items-center justify-between gap-3">
              <span>Local checkouts</span>
              <span className="font-medium text-foreground">
                {localRepositoryCount}
              </span>
            </div>
          </div>
        </div>
        <div className="border-border/50 border-t pt-3">
          <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Scope
          </h3>
          <p className="mt-2 text-sm text-muted-foreground">
            Showing project announcements, repository metadata, and activity
            visible in the current workspace.
          </p>
        </div>
      </aside>
    </section>
  );
}
