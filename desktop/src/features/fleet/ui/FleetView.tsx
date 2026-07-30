import * as React from "react";
import { Bot } from "lucide-react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useWorkingChannels } from "@/features/agents/agentWorkingSignal";
import { useOpenAgentActivity } from "@/features/agents/useOpenAgentActivity";
import { useChannelsQuery } from "@/features/channels/hooks";
import { usePresenceQuery } from "@/features/presence/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { PageHeader } from "@/shared/ui/PageHeader";
import { Skeleton } from "@/shared/ui/skeleton";
import type { FleetAgent } from "../fleetAgents";
import { useFleetAgents } from "../useFleetAgents";
import {
  deriveFleetLiveStatus,
  formatTokenCount,
  formatUsdCost,
  type FleetLiveStatus,
} from "../fleetStatus";
import {
  getAgentUsageTotals,
  useFleetMetricsVersion,
} from "../turnMetricsStore";
import { displayTokenTotal } from "../turnMetrics";
import { FleetAgentCard } from "./FleetAgentCard";

export function FleetView() {
  const { agents, error, isLoading } = useFleetAgents();
  const { openAgentActivity } = useOpenAgentActivity();
  const { goAgents } = useAppNavigation();

  const agentPubkeys = React.useMemo(
    () => agents.map((agent) => agent.pubkey),
    [agents],
  );
  const presenceQuery = usePresenceQuery(agentPubkeys);
  const presenceLookup = presenceQuery.data ?? {};

  const channelsQuery = useChannelsQuery();
  const channelIdToName = React.useMemo(() => {
    const map: Record<string, string> = {};
    for (const channel of channelsQuery.data ?? []) {
      map[channel.id] = channel.name;
    }
    return map;
  }, [channelsQuery.data]);

  const handleOpen = React.useCallback(
    (pubkey: string) => {
      openAgentActivity(pubkey);
    },
    [openAgentActivity],
  );

  return (
    <div
      className="flex-1 overflow-y-auto overflow-x-hidden overscroll-contain px-4 py-7 sm:px-6 sm:py-8"
      data-testid="fleet-view"
    >
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-6">
        <PageHeader
          description="Every agent you own, at a glance — live status, current activity, and usage."
          title="Agent Fleet"
        />
        {isLoading ? (
          <FleetLoadingSkeleton />
        ) : error ? (
          <p className="text-sm text-destructive">
            Could not load agents: {error.message}
          </p>
        ) : agents.length === 0 ? (
          <FleetEmptyState onOpenAgents={() => void goAgents()} />
        ) : (
          <>
            <FleetSummary agents={agents} />
            <div
              className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3"
              data-testid="fleet-grid"
            >
              {agents.map((agent) => (
                <FleetAgentCard
                  agent={agent}
                  channelIdToName={channelIdToName}
                  key={agent.pubkey}
                  onOpen={handleOpen}
                  presenceLoaded={presenceQuery.isFetched}
                  presenceStatus={
                    presenceLookup[agent.pubkey.trim().toLowerCase()]
                  }
                />
              ))}
            </div>
          </>
        )}
      </div>
    </div>
  );
}

const STATUS_SUMMARY_ORDER: Array<{
  status: FleetLiveStatus;
  label: string;
  dotClassName: string;
}> = [
  { status: "working", label: "working", dotClassName: "bg-primary" },
  { status: "idle", label: "idle", dotClassName: "bg-muted-foreground/50" },
  { status: "error", label: "need attention", dotClassName: "bg-destructive" },
  { status: "offline", label: "offline", dotClassName: "bg-border" },
];

function FleetSummary({ agents }: { agents: FleetAgent[] }) {
  // One subscription for the whole strip: the working signal drives status
  // counts, the metrics version bumps whenever new turn metrics arrive.
  const workingChannels = useWorkingChannels();
  const metricsVersion = useFleetMetricsVersion();

  const workingSet = React.useMemo(() => {
    const set = new Set<string>();
    for (const channel of workingChannels) {
      for (const pubkey of channel.agentPubkeys) {
        set.add(normalizePubkey(pubkey));
      }
    }
    return set;
  }, [workingChannels]);

  const statusCounts = React.useMemo(() => {
    const counts: Record<FleetLiveStatus, number> = {
      working: 0,
      idle: 0,
      error: 0,
      offline: 0,
    };
    for (const agent of agents) {
      counts[
        deriveFleetLiveStatus({
          working: workingSet.has(normalizePubkey(agent.pubkey)),
          status: agent.status,
          lastError: agent.lastError,
        })
      ] += 1;
    }
    return counts;
  }, [agents, workingSet]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: metricsVersion is the store-change signal that invalidates getAgentUsageTotals reads
  const usage = React.useMemo(() => {
    let tokens: number | null = null;
    let cost: number | null = null;
    for (const agent of agents) {
      const totals = getAgentUsageTotals(agent.pubkey);
      const agentTokens = displayTokenTotal(totals);
      if (agentTokens !== null) tokens = (tokens ?? 0) + agentTokens;
      if (totals.costUsd !== null) cost = (cost ?? 0) + totals.costUsd;
    }
    return { tokens, cost };
  }, [agents, metricsVersion]);

  return (
    <div
      className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 rounded-lg border border-border/50 bg-muted/20 px-4 py-2.5"
      data-testid="fleet-summary"
    >
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm text-muted-foreground">
        {STATUS_SUMMARY_ORDER.map(({ status, label, dotClassName }) =>
          statusCounts[status] > 0 ? (
            <span
              className="inline-flex items-center gap-1.5"
              data-testid={`fleet-summary-${status}`}
              key={status}
            >
              <span
                aria-hidden
                className={`h-2 w-2 rounded-full ${dotClassName}`}
              />
              {statusCounts[status]} {label}
            </span>
          ) : null,
        )}
      </div>
      {usage.tokens !== null || usage.cost !== null ? (
        <span className="text-sm text-muted-foreground">
          {usage.tokens !== null
            ? `${formatTokenCount(usage.tokens)} tokens`
            : null}
          {usage.tokens !== null && usage.cost !== null ? " · " : null}
          {usage.cost !== null ? formatUsdCost(usage.cost) : null}
        </span>
      ) : null}
    </div>
  );
}

function FleetEmptyState({ onOpenAgents }: { onOpenAgents: () => void }) {
  return (
    <div className="flex min-h-64 flex-col items-center justify-center gap-3 rounded-lg border border-dashed border-border/70 px-6 py-10 text-center">
      <Bot aria-hidden className="h-6 w-6 text-muted-foreground" />
      <div>
        <p className="text-sm font-medium">No agents yet</p>
        <p className="mt-1 text-sm text-muted-foreground">
          Create an agent and it will show up here with live status and usage.
        </p>
      </div>
      <Button onClick={onOpenAgents} size="sm" variant="outline">
        Open Agents
      </Button>
    </div>
  );
}

function FleetLoadingSkeleton() {
  return (
    <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
      {[0, 1, 2].map((index) => (
        <div
          className="flex flex-col gap-3 rounded-lg border border-border/70 p-4"
          key={index}
        >
          <div className="flex items-center gap-3">
            <Skeleton className="h-9 w-9 rounded-full" />
            <div className="flex-1 space-y-1.5">
              <Skeleton className="h-4 w-28 rounded" />
              <Skeleton className="h-3 w-20 rounded" />
            </div>
          </div>
          <Skeleton className="h-4 w-full rounded" />
          <Skeleton className="h-3 w-2/3 rounded" />
        </div>
      ))}
    </div>
  );
}
