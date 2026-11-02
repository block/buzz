import { Link } from "@tanstack/react-router";

import { useCompanyTeamQuery } from "@/features/company-team/teamRelay";
import type { TeamMember } from "@/features/company-team/teamModels";
import { EmployeeAvatar } from "@/features/company-team/ui/EmployeeAvatar";
import { TeamPage, TeamPageTitle } from "@/features/company-team/ui/TeamPage";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { WorkspaceMark } from "@/features/communities/ui/WorkspaceMark";
import { useCommunities } from "@/features/communities/useCommunities";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";

import { FUNCTION_DETAILS, FUNCTION_ORDER } from "../functions";
import { useCofounderSettingsQuery, useFunctionSlotsQuery } from "../hooks";
import type { FunctionId, FunctionSlotView } from "../cofounderRecords";

type ProfileLookup = Record<
  string,
  { displayName: string | null; avatarUrl?: string | null }
>;

const NODE_POSITION: Record<FunctionId, { x: number; y: number }> = {
  sales: { x: 82, y: 33 },
  operations: { x: 82, y: 68 },
  customer_success: { x: 50, y: 85 },
  finance: { x: 18, y: 68 },
  brand: { x: 18, y: 33 },
};

function displayName(member: TeamMember, profiles: ProfileLookup): string {
  return (
    profiles[member.pubkey.toLowerCase()]?.displayName?.trim() ||
    member.managedAgent?.name?.trim() ||
    member.fallbackName?.trim() ||
    "Company agent"
  );
}

function agentStatusClass(member: TeamMember | null): string {
  if (
    member?.relayAgent?.status === "online" ||
    member?.managedAgent?.status === "running"
  ) {
    return "bg-emerald-500";
  }
  if (member?.relayAgent?.status === "away") return "bg-amber-500";
  return "bg-muted-foreground/50";
}

function MapFunctionNode({
  slot,
  member,
  name,
  avatarUrl,
}: {
  slot: FunctionSlotView;
  member: TeamMember | null;
  name: string | null;
  avatarUrl: string | null;
}) {
  const detail = FUNCTION_DETAILS[slot.functionId];
  const position = NODE_POSITION[slot.functionId];
  const covered = slot.state === "covered_by_scout";
  const filled = slot.state === "filled" && member !== null && name !== null;
  const description = covered
    ? `${detail.name}, covered by Scout`
    : filled
      ? `${detail.name}, led by ${name}`
      : `${detail.name}, empty slot`;

  return (
    <Link
      aria-label={description}
      className="group absolute flex -translate-x-1/2 -translate-y-1/2 flex-col items-center gap-2 rounded-xl text-center outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-4"
      data-function={slot.functionId}
      data-state={slot.state}
      data-testid={`cofounder-map-${slot.functionId}`}
      style={{ left: `${position.x}%`, top: `${position.y}%` }}
      to="/company/function/$function"
      params={{ function: slot.functionId }}
    >
      {filled ? (
        <span className="relative block size-[4.5rem] rounded-full ring-2 ring-primary shadow-lg transition-transform group-hover:scale-105 sm:size-24 lg:size-32">
          {member.kind === "employee" ? (
            <EmployeeAvatar
              className="size-full rounded-full"
              instance={member.managedAgent?.avatarUrl}
              name={name}
              pubkey={member.pubkey}
              personaId={member.managedAgent?.personaId}
              profile={avatarUrl}
            />
          ) : (
            <ProfileAvatar
              avatarUrl={avatarUrl}
              className="size-full rounded-full"
              label={name}
              pubkey={member.pubkey}
            />
          )}
          <span
            aria-hidden="true"
            className={`absolute bottom-0.5 right-0.5 size-3.5 rounded-full ring-[3px] ring-background ${agentStatusClass(member)}`}
          />
        </span>
      ) : covered ? (
        <span className="relative block size-[4.5rem] rounded-full bg-primary/10 ring-2 ring-primary shadow-md transition-transform group-hover:scale-105 sm:size-24 lg:size-32">
          <EmployeeAvatar className="size-full rounded-full" name="Scout" />
          <span
            aria-hidden="true"
            className="absolute inset-0 -rotate-45 rounded-full border-[3px] border-primary border-b-transparent border-l-transparent"
          />
          <span
            aria-hidden="true"
            className={`absolute bottom-0.5 right-0.5 size-3.5 rounded-full ring-[3px] ring-background ${agentStatusClass(member)}`}
          />
        </span>
      ) : (
        <span className="grid size-[4.5rem] place-items-center rounded-full border-2 border-dashed border-primary/30 text-muted-foreground transition-colors group-hover:border-primary group-hover:text-primary sm:size-24 lg:size-32">
          <svg
            aria-hidden="true"
            fill="none"
            height="1.5rem"
            viewBox="0 0 24 24"
            width="1.5rem"
          >
            <path
              d="M12 5v14M5 12h14"
              stroke="currentColor"
              strokeLinecap="round"
              strokeWidth="1.8"
            />
          </svg>
        </span>
      )}
      <span className="max-w-32 text-sm font-extrabold leading-tight text-foreground sm:text-base">
        {detail.name}
      </span>
      {covered ? (
        <span className="-mt-1 text-xs font-semibold text-muted-foreground">
          Covered by Scout
        </span>
      ) : null}
    </Link>
  );
}

export function CompanyMapScreen() {
  const { activeCommunity } = useCommunities();
  const settingsQuery = useCofounderSettingsQuery(activeCommunity?.id ?? null);
  const slots = useFunctionSlotsQuery(activeCommunity?.id ?? null);
  const teamQuery = useCompanyTeamQuery(Boolean(activeCommunity));
  const members = teamQuery.data?.members ?? [];
  const profilePubkeys = members.map((member) => member.pubkey);
  const profilesQuery = useUsersBatchQuery(profilePubkeys, {
    enabled: profilePubkeys.length > 0,
  });
  const profiles: ProfileLookup = profilesQuery.data?.profiles ?? {};
  const scout = members.find(
    (member) => displayName(member, profiles).toLowerCase() === "scout",
  );
  const companyName = activeCommunity?.name ?? "Company";

  if (settingsQuery.isLoading || teamQuery.isLoading) {
    return (
      <TeamPage testId="cofounder-company-map" title="Company map">
        <p aria-live="polite" className="text-sm text-muted-foreground">
          Loading the company map
        </p>
      </TeamPage>
    );
  }
  if (settingsQuery.isError || teamQuery.isError) {
    return (
      <TeamPage testId="cofounder-company-map" title="Company map">
        <div role="alert" className="space-y-3">
          <TeamPageTitle>Company map</TeamPageTitle>
          <p className="text-sm text-destructive">
            The company map could not load.
          </p>
          <button
            className="rounded-lg border border-border px-3 py-2 text-sm font-semibold hover:bg-muted focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary"
            onClick={() => {
              void settingsQuery.refetch();
              void teamQuery.refetch();
            }}
            type="button"
          >
            Try again
          </button>
        </div>
      </TeamPage>
    );
  }
  if (!settingsQuery.data?.settings.enabled) {
    return (
      <TeamPage testId="cofounder-company-map" title="Company map">
        <div className="space-y-3">
          <TeamPageTitle>Company map</TeamPageTitle>
          <p className="text-sm text-muted-foreground">
            Co-founder mode is off for {companyName}.
          </p>
          <Link
            className="inline-flex items-center rounded-xl border border-border px-4 py-2 text-sm font-bold hover:bg-muted"
            to="/settings"
          >
            Open settings
          </Link>
        </div>
      </TeamPage>
    );
  }

  const slotByFunction = new Map(slots.map((slot) => [slot.functionId, slot]));
  return (
    <TeamPage testId="cofounder-company-map" title="Company map">
      <header className="flex flex-wrap items-end justify-between gap-4">
        <div className="team-head-copy">
          <TeamPageTitle>Company map</TeamPageTitle>
          <p className="team-subtitle">How {companyName} is run.</p>
        </div>
        <Link
          className="inline-flex h-11 items-center rounded-xl border border-border px-4 text-sm font-bold hover:bg-muted focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary"
          to="/company/weekly-review"
        >
          Weekly review
        </Link>
      </header>
      <section
        aria-label={`${companyName} company map`}
        className="relative mx-auto h-[min(62vh,38rem)] min-h-[28rem] w-full max-w-[66rem] sm:min-h-[32rem]"
        data-testid="cofounder-map-stage"
      >
        <svg
          aria-hidden="true"
          className="absolute inset-0 size-full"
          preserveAspectRatio="none"
          viewBox="0 0 1000 650"
        >
          <path
            className="text-primary"
            d="M500 200V332"
            fill="none"
            stroke="currentColor"
            strokeWidth="4"
          />
          {FUNCTION_ORDER.map((functionId) => {
            const slot = slotByFunction.get(functionId) ?? {
              functionId,
              state: "empty" as const,
              leadPubkey: null,
            };
            const { x, y } = NODE_POSITION[functionId];
            const startX = 500;
            const startY = 332;
            const endX = x * 10;
            const endY = y * 6.5;
            const direction = endX < startX ? -1 : endX > startX ? 1 : 0;
            const path =
              direction === 0
                ? `M${startX} ${startY}V${endY}`
                : `M${startX} ${startY}C${startX + direction * 115} ${startY}, ${endX - direction * 60} ${endY - (endY > startY ? 18 : -18)}, ${endX} ${endY}`;
            return (
              <path
                className={
                  slot.state === "empty"
                    ? "text-primary/25"
                    : slot.state === "covered_by_scout"
                      ? "text-primary/55"
                      : "text-primary"
                }
                d={path}
                fill="none"
                key={functionId}
                stroke="currentColor"
                strokeDasharray={slot.state === "empty" ? "10 12" : undefined}
                strokeWidth="4"
              />
            );
          })}
        </svg>

        <figure
          aria-label="Scout, Chief of Staff"
          className="absolute flex -translate-x-1/2 -translate-y-1/2 flex-col items-center gap-1"
          data-testid="cofounder-map-scout"
          style={{ left: "50%", top: "31%" }}
        >
          <span className="relative block size-[4.5rem] rounded-full bg-primary/10 ring-2 ring-primary shadow-md sm:size-20">
            <EmployeeAvatar className="size-full rounded-full" name="Scout" />
            <span
              aria-hidden="true"
              className={`absolute bottom-0.5 right-0.5 size-3 rounded-full ring-2 ring-background ${agentStatusClass(scout ?? null)}`}
            />
          </span>
          <span className="text-sm font-extrabold">Scout</span>
          <span className="-mt-1 text-xs text-muted-foreground">
            Chief of Staff
          </span>
        </figure>

        <WorkspaceMark
          className="absolute z-10 grid size-[4.5rem] -translate-x-1/2 -translate-y-1/2 place-items-center overflow-hidden rounded-[1.25rem] bg-[#24212a] text-2xl font-black text-white shadow-xl sm:size-20 sm:text-3xl"
          communityIconFit="cover"
          communityId={activeCommunity?.id}
          dataTestId="cofounder-map-company-logo"
          fallbackContent={
            <span>
              {companyName.trim().slice(0, 1).toUpperCase() || "C"}
              <i className="text-primary not-italic">.</i>
            </span>
          }
          imageFit="contain"
          name={companyName}
          relayUrl={activeCommunity?.relayUrl}
          style={{ left: "50%", top: "51%" }}
        />

        {FUNCTION_ORDER.map((functionId) => {
          const slot = slotByFunction.get(functionId) ?? {
            functionId,
            state: "empty" as const,
            leadPubkey: null,
          };
          const member =
            slot.state === "filled" && slot.leadPubkey
              ? (members.find(
                  (candidate) =>
                    candidate.pubkey.toLowerCase() ===
                    slot.leadPubkey?.toLowerCase(),
                ) ?? null)
              : null;
          const name = member ? displayName(member, profiles) : null;
          const avatarUrl = member
            ? (profiles[member.pubkey.toLowerCase()]?.avatarUrl ?? null)
            : null;
          return (
            <MapFunctionNode
              avatarUrl={avatarUrl}
              key={functionId}
              member={member}
              name={name}
              slot={slot}
            />
          );
        })}
      </section>
    </TeamPage>
  );
}
