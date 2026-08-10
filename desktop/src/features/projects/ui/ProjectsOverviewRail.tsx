import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type {
  Project,
  ProjectActivitySummary,
} from "@/features/projects/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { OverviewRailSection } from "./ProjectOverviewPanel";
import { ProjectsContributionGraph } from "./ProjectsContributionGraph";

type ProjectsOverviewRailProps = {
  profiles?: UserProfileLookup;
  projects: Project[];
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
          ...project.repositories.flatMap((repository) => [
            repository.owner,
            ...repository.contributors,
          ]),
          ...(summaries?.[project.id]?.participantPubkeys ?? []),
        ].map(normalizePubkey),
      ),
    ),
  ];
}

function overviewActivityByDay(
  projects: Project[],
  summaries: Record<string, ProjectActivitySummary> | undefined,
) {
  const merged: Record<string, number> = {};
  for (const project of projects) {
    const byDay = summaries?.[project.id]?.activityByDay;
    if (!byDay) continue;
    for (const [day, count] of Object.entries(byDay)) {
      merged[day] = (merged[day] ?? 0) + count;
    }
  }
  return merged;
}

/** Workspace people and contribution activity for the overview side rail. */
export function ProjectsOverviewRail({
  profiles,
  projects,
  summaries,
}: ProjectsOverviewRailProps) {
  const people = overviewPeople(projects, summaries);
  const activityByDay = overviewActivityByDay(projects, summaries);

  // Plain stacked cards — the overview panel's rail column owns placement
  // and spacing, so the sections can never drift apart or collide.
  return (
    <>
      <div className="rounded-lg border border-border/60 p-4">
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

      <div className="min-w-0 rounded-lg border border-border/60 p-4">
        <OverviewRailSection
          title="Contribution Activity"
          titleClassName="text-base"
        >
          <ProjectsContributionGraph activityByDay={activityByDay} compact />
        </OverviewRailSection>
      </div>
    </>
  );
}
