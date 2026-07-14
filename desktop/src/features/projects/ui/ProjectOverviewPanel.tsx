import {
  FileCode2,
  GitBranch,
  GitCommitHorizontal,
  GitPullRequest,
  Users,
} from "lucide-react";
import type * as React from "react";

import type {
  Project,
  ProjectPullRequest,
  ProjectRepoContributor,
  ProjectRepoFile,
  ProjectRepoSnapshot,
} from "@/features/projects/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import {
  LANGUAGE_DOT_CLASSES,
  languageForPath,
  topLanguagesFromCounts,
} from "@/features/projects/lib/projectLanguages";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { ReadmePanel } from "./ProjectReadmePanel";
import type { RepoSourceHeaderControls } from "./ProjectRepositorySource";

type ProjectOverviewPanelProps = {
  contributors: ProjectRepoContributor[];
  files: ProjectRepoFile[];
  project: Project;
  onViewContributors: () => void;
  profiles?: UserProfileLookup;
  pullRequests: ProjectPullRequest[];
  readmeFile: ProjectRepoFile | null;
  snapshot: ProjectRepoSnapshot | null | undefined;
  /** Branch picker + remote/local toggle for the readme header. */
  sourceControls?: RepoSourceHeaderControls;
};

function shortHash(hash: string | undefined) {
  return hash ? hash.slice(0, 7) : "None";
}

function topLanguages(files: ProjectRepoFile[]) {
  const counts: Record<string, number> = {};
  for (const file of files) {
    const language = languageForPath(file.path);
    if (language) counts[language] = (counts[language] ?? 0) + 1;
  }
  return topLanguagesFromCounts(counts);
}

function projectPeople(project: Project) {
  return [
    ...new Set(
      [project.owner, ...project.contributors]
        .filter(Boolean)
        .map(normalizePubkey),
    ),
  ];
}

function PeopleAvatars({
  people,
  profiles,
}: {
  people: string[];
  profiles?: UserProfileLookup;
}) {
  return (
    <div className="flex flex-wrap gap-1.5">
      {people.slice(0, 18).map((person) => {
        const profile = profiles?.[normalizePubkey(person)];
        const label =
          profile?.displayName?.trim() ||
          profile?.nip05Handle?.trim() ||
          person;
        return (
          <UserAvatar
            accent={profile?.isAgent === true}
            avatarUrl={profile?.avatarUrl ?? null}
            displayName={label}
            key={person}
            size="sm"
          />
        );
      })}
    </div>
  );
}

export function LanguageChips({
  languages,
}: {
  languages: Array<[string, number]>;
}) {
  return (
    <div className="flex flex-wrap gap-1.5">
      {languages.map(([language], index) => (
        <span
          className="inline-flex items-center gap-1.5 rounded-full border border-border/60 bg-muted/40 px-2 py-1 text-xs text-muted-foreground"
          key={language}
        >
          <span
            className={`h-2 w-2 rounded-full ${
              LANGUAGE_DOT_CLASSES[index % LANGUAGE_DOT_CLASSES.length]
            }`}
          />
          {language}
        </span>
      ))}
    </div>
  );
}

export function OverviewRailSection({
  children,
  title,
}: {
  children: React.ReactNode;
  title: string;
}) {
  return (
    <section className="space-y-2">
      <h3 className="text-sm font-semibold text-foreground">{title}</h3>
      {children}
    </section>
  );
}

export function ProjectOverviewPanel({
  contributors,
  files,
  onViewContributors,
  project,
  profiles,
  pullRequests,
  readmeFile,
  snapshot,
  sourceControls,
}: ProjectOverviewPanelProps) {
  const languages = topLanguages(files);
  const people = projectPeople(project);
  const latestCommit = snapshot?.latestCommit ?? null;

  return (
    <div className="grid overflow-hidden rounded-xl border border-border/60 bg-card xl:grid-cols-[minmax(0,1fr)_18rem]">
      <div className="min-w-0">
        {/* ReadmePanel renders its own "no README" fallback while keeping
            the branch + source controls reachable. */}
        <ReadmePanel file={readmeFile} sourceControls={sourceControls} />
      </div>
      <aside className="space-y-6 border-t border-border/60 p-4 xl:border-l xl:border-t-0">
        <OverviewRailSection title="People">
          <div className="flex items-center justify-between gap-3">
            <PeopleAvatars people={people} profiles={profiles} />
            <button
              className="shrink-0 rounded-md text-xs font-medium text-primary hover:underline focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
              onClick={onViewContributors}
              type="button"
            >
              View all
            </button>
          </div>
        </OverviewRailSection>
        <OverviewRailSection title="Top Languages">
          {languages.length > 0 ? (
            <LanguageChips languages={languages} />
          ) : (
            <p className="text-sm text-muted-foreground">
              No language data is available yet.
            </p>
          )}
        </OverviewRailSection>
        <OverviewRailSection title="Repository">
          <dl className="space-y-2 text-sm">
            <div className="flex items-center justify-between gap-3">
              <dt className="flex items-center gap-1.5 text-muted-foreground">
                <GitBranch className="h-3.5 w-3.5" />
                Branch
              </dt>
              <dd className="font-medium text-foreground">
                {project.defaultBranch}
              </dd>
            </div>
            <div className="flex items-center justify-between gap-3">
              <dt className="flex items-center gap-1.5 text-muted-foreground">
                <GitCommitHorizontal className="h-3.5 w-3.5" />
                Latest
              </dt>
              <dd className="font-mono text-xs text-foreground">
                {shortHash(latestCommit?.hash)}
              </dd>
            </div>
            <div className="flex items-center justify-between gap-3">
              <dt className="flex items-center gap-1.5 text-muted-foreground">
                <FileCode2 className="h-3.5 w-3.5" />
                Files
              </dt>
              <dd className="font-medium text-foreground">{files.length}</dd>
            </div>
            <div className="flex items-center justify-between gap-3">
              <dt className="flex items-center gap-1.5 text-muted-foreground">
                <Users className="h-3.5 w-3.5" />
                Contributors
              </dt>
              <dd className="font-medium text-foreground">
                {contributors.length}
              </dd>
            </div>
            <div className="flex items-center justify-between gap-3">
              <dt className="flex items-center gap-1.5 text-muted-foreground">
                <GitPullRequest className="h-3.5 w-3.5" />
                Pull Requests
              </dt>
              <dd className="font-medium text-foreground">
                {pullRequests.length}
              </dd>
            </div>
          </dl>
        </OverviewRailSection>
      </aside>
    </div>
  );
}
