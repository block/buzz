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
          ...project.contributors,
          ...(summaries?.[project.repoAddress]?.participantPubkeys ?? []),
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
    const byDay = summaries?.[project.repoAddress]?.activityByDay;
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

      <div className="order-5 min-w-0 border-t border-border/40 px-4 pb-4 pt-3 xl:order-none xl:col-start-2 xl:row-start-2 xl:border-t-0">
        <section className="space-y-2">
          <h3 className="text-base font-semibold text-foreground">
            Contribution Activity
          </h3>
          <ProjectsContributionGraph activityByDay={activityByDay} compact />
        </section>
      </div>
    </>
  );
}
