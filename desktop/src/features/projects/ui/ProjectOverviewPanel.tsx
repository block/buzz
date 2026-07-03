import { FileCode2, GitBranch, GitCommitHorizontal, Users } from "lucide-react";
import type * as React from "react";

import type {
  Project,
  ProjectPullRequest,
  ProjectRepoContributor,
  ProjectRepoFile,
  ProjectRepoSnapshot,
} from "@/features/projects/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { ReadmePanel } from "./ProjectRepositoryPanel";

type ProjectOverviewPanelProps = {
  contributors: ProjectRepoContributor[];
  files: ProjectRepoFile[];
  project: Project;
  onViewContributors: () => void;
  profiles?: UserProfileLookup;
  pullRequests: ProjectPullRequest[];
  readmeFile: ProjectRepoFile | null;
  snapshot: ProjectRepoSnapshot | null | undefined;
};

const LANGUAGE_LABELS: Record<string, string> = {
  css: "CSS",
  dart: "Dart",
  go: "Go",
  html: "HTML",
  js: "JavaScript",
  json: "JSON",
  jsx: "JavaScript",
  kt: "Kotlin",
  mjs: "JavaScript",
  py: "Python",
  rb: "Ruby",
  rs: "Rust",
  swift: "Swift",
  ts: "TypeScript",
  tsx: "TypeScript",
};

const LANGUAGE_DOT_CLASSES = [
  "bg-blue-500",
  "bg-violet-500",
  "bg-emerald-500",
  "bg-orange-500",
  "bg-pink-500",
];

function shortHash(hash: string | undefined) {
  return hash ? hash.slice(0, 7) : "None";
}

function languageForPath(path: string) {
  const fileName = path.split("/").pop()?.toLowerCase() ?? "";
  const extension = fileName.includes(".") ? fileName.split(".").pop() : "";
  return extension ? LANGUAGE_LABELS[extension] : undefined;
}

function topLanguages(files: ProjectRepoFile[]) {
  const counts = new Map<string, number>();
  for (const file of files) {
    const language = languageForPath(file.path);
    if (language) counts.set(language, (counts.get(language) ?? 0) + 1);
  }
  return [...counts.entries()]
    .sort(
      (left, right) => right[1] - left[1] || left[0].localeCompare(right[0]),
    )
    .slice(0, 5);
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

export function OverviewRailSection({
  children,
  title,
}: {
  children: React.ReactNode;
  title: string;
}) {
  return (
    <section className="space-y-2 border-border/50 border-b pb-4 last:border-b-0 last:pb-0">
      <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        {title}
      </h3>
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
}: ProjectOverviewPanelProps) {
  const languages = topLanguages(files);
  const people = projectPeople(project);
  const latestCommit = snapshot?.latestCommit ?? null;

  return (
    <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_18rem]">
      <div className="min-w-0">
        {readmeFile ? (
          <ReadmePanel file={readmeFile} />
        ) : (
          <div className="rounded-xl border border-border/50 bg-card/60 p-6 text-sm text-muted-foreground">
            <p className="font-medium text-foreground">No README found</p>
            <p className="mt-1">
              Add a README to this repository to describe the project and help
              collaborators get started.
            </p>
          </div>
        )}
      </div>
      <aside className="space-y-4 rounded-xl border border-border/50 bg-card/60 p-4">
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
            <div className="flex flex-wrap gap-1.5">
              {languages.map(([language], index) => (
                <span
                  className="inline-flex items-center gap-1.5 rounded-full bg-muted/70 px-2 py-1 text-xs text-muted-foreground"
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
              <dt className="text-muted-foreground">PRs</dt>
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
