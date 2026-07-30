import { AlertTriangle, Coins, Hash } from "lucide-react";

import { useAgentWorking } from "@/features/agents/agentWorkingSignal";
import { friendlyAgentLastError } from "@/features/agents/lib/friendlyAgentLastError";
import { AgentStatusBadge } from "@/features/agents/ui/AgentStatusBadge";
import { IdentityInitialsAvatar } from "@/features/agents/ui/IdentityInitialsAvatar";
import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import {
  useAgentTranscript,
  useObserverEvents,
} from "@/features/agents/ui/useObserverEvents";
import type { PresenceStatus } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { useNow } from "@/shared/lib/useNow";
import type { FleetAgent } from "../fleetAgents";
import {
  deriveActivityHeadline,
  deriveFleetLiveStatus,
  deriveLastSeenMs,
  formatLastSeen,
  formatTokenCount,
  formatUsdCost,
} from "../fleetStatus";
import { useAgentUsageTotals } from "../turnMetricsStore";
import { displayTokenTotal } from "../turnMetrics";

const LAST_SEEN_TICK_MS = 30_000;
const ELAPSED_TICK_MS = 1_000;

export function FleetAgentCard({
  agent,
  channelIdToName,
  onOpen,
  presenceLoaded,
  presenceStatus,
}: {
  agent: FleetAgent;
  channelIdToName: Record<string, string>;
  onOpen: (pubkey: string) => void;
  presenceLoaded: boolean;
  presenceStatus: PresenceStatus | undefined;
}) {
  // Store reads only — the app-shell observer ingestion
  // (`useAgentObserverIngestion`) already owns the relay subscription, so the
  // `enabled` flag stays false here to avoid a redundant ensure.
  const { events } = useObserverEvents(false, agent.pubkey);
  const transcript = useAgentTranscript(false, agent.pubkey);
  const workingState = useAgentWorking(agent.pubkey);
  const usage = useAgentUsageTotals(agent.pubkey);

  const liveStatus = deriveFleetLiveStatus({
    working: workingState.working,
    status: agent.status,
    lastError: agent.lastError,
  });
  const headline = deriveActivityHeadline(transcript);
  const lastSeenMs = deriveLastSeenMs({
    events,
    lastStartedAt: agent.lastStartedAt,
    lastStoppedAt: agent.lastStoppedAt,
  });
  const friendlyError =
    liveStatus === "error"
      ? (friendlyAgentLastError(agent.lastError, agent.lastErrorCode)?.copy ??
        null)
      : null;
  const tokenTotal = displayTokenTotal(usage);

  return (
    <button
      aria-label={`Open ${agent.name} activity`}
      className={cn(
        "flex min-w-0 flex-col gap-3 rounded-lg border border-border/70 bg-background/80 p-4 text-left shadow-xs transition-colors",
        "hover:border-border hover:bg-muted/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        liveStatus === "error" && "border-destructive/40",
      )}
      data-status={liveStatus}
      data-testid={`fleet-agent-card-${agent.pubkey}`}
      onClick={() => onOpen(agent.pubkey)}
      type="button"
    >
      <div className="flex items-center gap-3">
        <span className="relative h-9 w-9 shrink-0">
          {agent.avatarUrl ? (
            <img
              alt=""
              className="h-full w-full rounded-full object-cover"
              src={agent.avatarUrl}
            />
          ) : (
            <IdentityInitialsAvatar
              className="border-0 text-sm shadow-none"
              colorSeed={agent.pubkey}
              label={agent.name}
              size={36}
            />
          )}
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-semibold">
            {agent.name}
          </span>
          <span className="block truncate text-2xs text-muted-foreground">
            {agent.isManaged
              ? (agent.model ?? "Local agent")
              : "Runs on another device"}
          </span>
        </span>
        <AgentStatusBadge
          isWorking={workingState.working}
          presenceLoaded={presenceLoaded}
          presenceStatus={presenceStatus}
          status={agent.status}
        />
      </div>

      {workingState.working ? (
        <WorkingChannels
          channelIdToName={channelIdToName}
          channels={workingState.channels}
        />
      ) : friendlyError ? (
        <p
          className="inline-flex min-w-0 items-center gap-1.5 text-sm text-destructive"
          data-testid="fleet-card-headline"
        >
          <AlertTriangle aria-hidden className="h-3.5 w-3.5 shrink-0" />
          <span className="truncate">{friendlyError}</span>
        </p>
      ) : (
        <p
          className="truncate text-sm text-muted-foreground"
          data-testid="fleet-card-headline"
        >
          {headline ?? "No recent activity"}
        </p>
      )}

      <div className="mt-auto flex items-center justify-between gap-2 border-t border-border/50 pt-2 text-2xs text-muted-foreground">
        <span className="inline-flex min-w-0 items-center gap-2.5">
          <span
            className="inline-flex items-center gap-1"
            title="Tokens across archived turn metrics"
          >
            <Hash aria-hidden className="h-3 w-3" />
            {tokenTotal !== null ? formatTokenCount(tokenTotal) : "—"}
          </span>
          <span
            className="inline-flex items-center gap-1"
            title="Estimated cost across archived turn metrics"
          >
            <Coins aria-hidden className="h-3 w-3" />
            {usage.costUsd !== null ? formatUsdCost(usage.costUsd) : "—"}
          </span>
          {usage.turnCount > 0 ? (
            <span>
              {usage.turnCount} turn{usage.turnCount === 1 ? "" : "s"}
            </span>
          ) : null}
        </span>
        <LastSeen lastSeenMs={lastSeenMs} />
      </div>
    </button>
  );
}

function WorkingChannels({
  channelIdToName,
  channels,
}: {
  channelIdToName: Record<string, string>;
  channels: ReturnType<typeof useAgentWorking>["channels"];
}) {
  // A live elapsed counter is displayed, so this component (and only this
  // component) mounts a ticking clock.
  const now = useNow(ELAPSED_TICK_MS);
  const visible = channels.slice(0, 2);
  const overflow = channels.length - visible.length;

  return (
    <div className="flex min-w-0 flex-wrap items-center gap-1.5">
      {visible.map((channel) => (
        <span
          className="inline-flex max-w-full items-center gap-1 rounded-full bg-primary/10 px-2 py-0.5 text-2xs text-primary"
          data-testid="fleet-card-working-channel"
          key={channel.channelId}
        >
          <span className="truncate">
            #{channelIdToName[channel.channelId] ?? channel.channelId}
          </span>
          <span className="shrink-0 font-mono">
            {formatElapsed(now - channel.anchorAt)}
          </span>
        </span>
      ))}
      {overflow > 0 ? (
        <span className="text-2xs text-muted-foreground">+{overflow} more</span>
      ) : null}
    </div>
  );
}

function LastSeen({ lastSeenMs }: { lastSeenMs: number | null }) {
  const now = useNow(LAST_SEEN_TICK_MS);
  if (lastSeenMs === null) return null;
  return (
    <span className="shrink-0" data-testid="fleet-card-last-seen">
      {formatLastSeen(lastSeenMs, now)}
    </span>
  );
}
