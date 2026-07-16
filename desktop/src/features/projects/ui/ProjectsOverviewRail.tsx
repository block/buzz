import type { ReactNode } from "react";

import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type {
  Project,
  ProjectActivitySummary,
  ProjectRepoSnapshot,
} from "@/features/projects/hooks";
import {
  languageForPath,
  topLanguagesFromCounts,
} from "@/features/projects/lib/projectLanguages";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { LanguageChips, OverviewRailSection } from "./ProjectOverviewPanel";

type ProjectsOverviewRailProps = {
  children: ReactNode;
  profiles?: UserProfileLookup;
  projects: Project[];
  snapshots?: Record<string, ProjectRepoSnapshot>;
  snapshotsLoading?: boolean;
  summaries?: Record<string, ProjectActivitySummary>;
};

function overviewPeople(
  projects: Project[],
  summaries: Record<string, ProjectActivitySummary> | undefined,
) {
  return [
    ...new Set(
      projects.flatMap((project) =>
        [
          project.owner,
          ...project.contributors,
          ...(summaries?.[project.repoAddress]?.participantPubkeys ?? []),
        ].map(normalizePubkey),
      ),
    ),
  ];
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

/** Workspace people and language metadata for the overview side rail. */
export function ProjectsOverviewRail({
  children,
  profiles,
  projects,
  snapshots,
  snapshotsLoading,
  summaries,
}: ProjectsOverviewRailProps) {
  const people = overviewPeople(projects, summaries);
  const languages = overviewLanguages(snapshots);

  return (
    <>
      <div className="order-4 min-w-0 border-t border-border/40 px-4 py-4 xl:order-none xl:col-start-2 xl:row-start-1 xl:border-t-0 xl:pt-0">
        <OverviewRailSection title="People" titleClassName="text-base">
          {people.length > 0 ? (
            <div className="flex flex-wrap gap-1.5">
              {people.slice(0, 18).map((pubkey) => {
                const profile = profiles?.[normalizePubkey(pubkey)];
                const label = resolveUserLabel({ profiles, pubkey });
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
          ) : (
            <p className="text-sm text-muted-foreground">No people yet.</p>
          )}
        </OverviewRailSection>
      </div>

      <div className="order-5 min-w-0 space-y-7 border-t border-border/40 px-4 pb-4 pt-3 xl:order-none xl:col-start-2 xl:row-span-2 xl:row-start-2 xl:border-t-0">
        <OverviewRailSection title="Top Languages" titleClassName="text-base">
          {languages.length > 0 ? (
            <LanguageChips languages={languages} />
          ) : (
            <p className="text-sm text-muted-foreground">
              {snapshotsLoading
                ? "Scanning repositories..."
                : "No language data is available yet."}
            </p>
          )}
        </OverviewRailSection>

        <OverviewRailSection
          title="Active Repositories"
          titleClassName="text-base"
        >
          {children}
        </OverviewRailSection>
      </div>
    </>
  );
}
