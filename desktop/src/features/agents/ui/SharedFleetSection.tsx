import * as React from "react";

import { useRelayAgentsQuery } from "@/features/agents/hooks";
import { useTeamCatalogQuery } from "@/features/agents/teamCatalogHooks";
import {
  activeSharedFleetRows,
  buildSharedFleetRows,
  type SharedFleetRow,
} from "@/features/agents/lib/sharedFleetModel";
import type { TeamCatalogPublication } from "@/features/agents/lib/teamCatalogRelay";
import { getPresenceLabel } from "@/features/presence/lib/presence";
import { usePresenceQuery } from "@/features/presence/hooks";
import type { PresenceLookup, RelayAgent } from "@/shared/api/types";
import { Badge } from "@/shared/ui/badge";
import { SectionHeader } from "@/shared/ui/PageHeader";

type SharedFleetContentProps = {
  fleetError: Error | null;
  isFleetLoading: boolean;
  isTeamsLoading: boolean;
  rows: readonly SharedFleetRow[];
  teamError: Error | null;
  teams: readonly TeamCatalogPublication[];
};

function statusVariant(
  status: SharedFleetRow["status"],
): "success" | "warning" | "secondary" {
  if (status === "online") return "success";
  if (status === "away") return "warning";
  return "secondary";
}

function memberLabel(memberCount: number): string {
  return `${memberCount} member${memberCount === 1 ? "" : "s"}`;
}

export function currentSharedFleetRows(
  relayAgents: readonly RelayAgent[],
  presence: PresenceLookup | undefined,
  {
    relayAgentsAreCurrent,
    presenceIsCurrent,
  }: {
    relayAgentsAreCurrent: boolean;
    presenceIsCurrent: boolean;
  },
): SharedFleetRow[] {
  if (
    !relayAgentsAreCurrent ||
    (relayAgents.length > 0 && !presenceIsCurrent)
  ) {
    return [];
  }
  return activeSharedFleetRows(buildSharedFleetRows(relayAgents, presence));
}

export function currentSharedTeamCatalog(
  teams: readonly TeamCatalogPublication[],
  teamCatalogIsCurrent: boolean,
): readonly TeamCatalogPublication[] {
  return teamCatalogIsCurrent ? teams : [];
}

/** Read-only rendering of relay-confirmed workers and public team heads. */
export function SharedFleetContent({
  fleetError,
  isFleetLoading,
  isTeamsLoading,
  rows,
  teamError,
  teams,
}: SharedFleetContentProps) {
  return (
    <div className="flex flex-col gap-8" data-testid="shared-fleet-section">
      <section className="space-y-4" data-testid="shared-fleet">
        <SectionHeader
          description="Relay-confirmed remote workers. Read-only: execution stays on the worker host."
          title="Shared fleet"
        />
        {isFleetLoading ? (
          <p className="text-sm text-muted-foreground">
            Loading live worker presence…
          </p>
        ) : rows.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No live shared workers are currently reported.
          </p>
        ) : (
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {rows.map((row) => (
              <article
                className="min-w-0 rounded-xl border border-border bg-card p-4 shadow-sm"
                data-testid={`shared-fleet-worker-${row.pubkey}`}
                key={row.pubkey}
              >
                <div className="flex min-w-0 items-start justify-between gap-3">
                  <div className="min-w-0">
                    <p className="truncate text-base font-semibold tracking-tight">
                      {row.name}
                    </p>
                    <p className="mt-0.5 text-xs text-muted-foreground">
                      Remote worker
                    </p>
                    {row.modelLabel ? (
                      <p className="mt-1 truncate text-xs text-muted-foreground">
                        {row.modelLabel}
                      </p>
                    ) : null}
                  </div>
                  <Badge variant={statusVariant(row.status)}>
                    {getPresenceLabel(row.status)}
                  </Badge>
                </div>
                <div className="mt-4 space-y-2">
                  <p className="text-xs font-medium text-muted-foreground">
                    Assigned channels
                  </p>
                  {row.channels.length === 0 ? (
                    <p className="text-sm text-muted-foreground">
                      No assigned channels
                    </p>
                  ) : (
                    <div className="flex flex-wrap gap-1.5">
                      {row.channels.map((channel) => (
                        <Badge
                          key={`${row.pubkey}:${channel}`}
                          variant="outline"
                        >
                          {channel}
                        </Badge>
                      ))}
                    </div>
                  )}
                  <p className="pt-1 text-xs text-muted-foreground">
                    {row.mentionHint}
                  </p>
                </div>
              </article>
            ))}
          </div>
        )}
        {fleetError ? (
          <p className="text-sm text-destructive" role="alert">
            {fleetError.message}
          </p>
        ) : null}
      </section>

      <section className="space-y-4" data-testid="shared-team-catalog">
        <SectionHeader
          description="Relay-confirmed shared teams. Membership and lifecycle stay with the team owner."
          title="Shared agent teams"
        />
        {isTeamsLoading ? (
          <p className="text-sm text-muted-foreground">Loading shared teams…</p>
        ) : teams.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No shared teams are currently published.
          </p>
        ) : (
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            {teams.map((team, index) => (
              <article
                className="min-w-0 rounded-xl border border-border bg-card p-4 shadow-sm"
                data-testid={`shared-team-card-${index}`}
                key={`${team.ownerPubkey}:${team.teamDTag}`}
              >
                <p className="truncate text-base font-semibold tracking-tight">
                  {team.name}
                </p>
                <p className="mt-1 text-sm text-muted-foreground">
                  {memberLabel(team.memberCount)}
                </p>
              </article>
            ))}
          </div>
        )}
        {teamError ? (
          <p className="text-sm text-destructive" role="alert">
            {teamError.message}
          </p>
        ) : null}
      </section>
    </div>
  );
}

export function SharedFleetSection() {
  const relayAgentsQuery = useRelayAgentsQuery();
  const teamCatalogQuery = useTeamCatalogQuery();
  const relayAgents = relayAgentsQuery.data ?? [];
  const pubkeys = React.useMemo(
    () => relayAgents.map((agent) => agent.pubkey),
    [relayAgents],
  );
  const presenceQuery = usePresenceQuery(pubkeys, {
    enabled: relayAgentsQuery.isSuccess && pubkeys.length > 0,
  });
  const rows = React.useMemo(
    () =>
      currentSharedFleetRows(relayAgents, presenceQuery.data, {
        relayAgentsAreCurrent:
          relayAgentsQuery.isSuccess && !relayAgentsQuery.isError,
        presenceIsCurrent: presenceQuery.isSuccess && !presenceQuery.isError,
      }),
    [
      presenceQuery.data,
      presenceQuery.isError,
      presenceQuery.isSuccess,
      relayAgents,
      relayAgentsQuery.isError,
      relayAgentsQuery.isSuccess,
    ],
  );
  const teams = currentSharedTeamCatalog(
    teamCatalogQuery.data ?? [],
    teamCatalogQuery.isSuccess && !teamCatalogQuery.isError,
  );
  const fleetError =
    relayAgentsQuery.error instanceof Error
      ? relayAgentsQuery.error
      : presenceQuery.error instanceof Error
        ? presenceQuery.error
        : null;
  const teamError =
    teamCatalogQuery.error instanceof Error ? teamCatalogQuery.error : null;

  return (
    <SharedFleetContent
      fleetError={fleetError}
      isFleetLoading={
        relayAgentsQuery.isLoading ||
        (relayAgents.length > 0 && presenceQuery.isLoading)
      }
      isTeamsLoading={teamCatalogQuery.isLoading}
      rows={rows}
      teamError={teamError}
      teams={teams}
    />
  );
}
