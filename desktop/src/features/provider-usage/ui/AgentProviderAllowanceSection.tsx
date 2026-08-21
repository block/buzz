import * as React from "react";
import { RefreshCw } from "lucide-react";

import { useAppShell } from "@/app/AppShellContext";
import { useAcpRuntimesQuery } from "@/features/agents/hooks";
import {
  constrainingProviderWindow,
  providerAllowanceLevel,
  resolveAgentProviderUsage,
  type ProviderAllowanceLevel,
} from "@/features/provider-usage/agentProviderUsage";
import { useProviderUsageSnapshot } from "@/features/provider-usage/hooks";
import {
  formatUsageReset,
  providerUsageErrorMessage,
} from "@/features/provider-usage/providerUsageDisplay.mjs";
import type { ManagedAgent } from "@/shared/api/types";
import type {
  ProviderUsageId,
  ProviderUsageWindow,
} from "@/shared/api/tauriProviderUsage";
import { Alert, AlertDescription } from "@/shared/ui/alert";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { SectionHeader } from "@/shared/ui/PageHeader";
import { Progress } from "@/shared/ui/progress";
import { Skeleton } from "@/shared/ui/skeleton";
import { cn } from "@/shared/lib/cn";

const allowancePresentation: Record<
  ProviderAllowanceLevel,
  {
    badge: "destructive" | "success" | "warning";
    label: string;
    progress: string;
  }
> = {
  healthy: {
    badge: "success",
    label: "Healthy",
    progress: "[&>div]:bg-emerald-500",
  },
  low: {
    badge: "warning",
    label: "Low",
    progress: "[&>div]:bg-amber-500",
  },
  critical: {
    badge: "destructive",
    label: "Critical",
    progress: "[&>div]:bg-destructive",
  },
  exhausted: {
    badge: "destructive",
    label: "Exhausted",
    progress: "[&>div]:bg-destructive",
  },
};

export function AgentProviderAllowanceSection({
  agents,
}: {
  agents: ManagedAgent[];
}) {
  const { onOpenSettings } = useAppShell();
  const runtimesQuery = useAcpRuntimesQuery();
  const snapshot = useProviderUsageSnapshot();
  const constrainingWindow = constrainingProviderWindow(
    snapshot.query.data?.windows ?? [],
  );
  const matchingAgentCount = React.useMemo(
    () =>
      agents.filter(
        (agent) =>
          resolveAgentProviderUsage(agent, runtimesQuery.data ?? [])
            .providerUsageId === snapshot.provider,
      ).length,
    [agents, runtimesQuery.data, snapshot.provider],
  );

  return (
    <section
      className="relative space-y-4"
      data-testid="agent-provider-allowance-section"
    >
      <SectionHeader
        action={
          snapshot.featureEnabled && snapshot.adapterAvailable ? (
            <Button
              disabled={snapshot.query.isFetching}
              onClick={() => void snapshot.query.refetch()}
              size="sm"
              variant="outline"
            >
              <RefreshCw
                aria-hidden="true"
                className={cn(
                  snapshot.query.isFetching && "motion-safe:animate-spin",
                )}
              />
              Refresh
            </Button>
          ) : null
        }
        description="Owner-private subscription limits read locally from provider tools. Never published to the relay."
        title="Provider allowance"
      />

      {!snapshot.featureEnabled ? (
        <Card className="p-6" data-testid="provider-allowance-disabled">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="space-y-1">
              <p className="text-sm font-medium">Provider reads are off</p>
              <p className="text-sm text-muted-foreground">
                Opt in to read exact allowance and reset timing from supported
                local provider tools.
              </p>
            </div>
            <Button
              onClick={() => onOpenSettings?.("experimental")}
              size="sm"
              variant="outline"
            >
              Open Experimental settings
            </Button>
          </div>
        </Card>
      ) : snapshot.capabilitiesQuery.isLoading || runtimesQuery.isLoading ? (
        <ProviderAllowanceSkeleton />
      ) : (
        <Card className="space-y-5 p-6" data-testid="provider-allowance-card">
          {snapshot.query.data && constrainingWindow ? (
            <ProviderAllowanceSummary
              matchingAgentCount={matchingAgentCount}
              planType={snapshot.query.data.planType}
              productLabel={snapshot.productLabel}
              queryFailed={snapshot.query.isError}
              window={constrainingWindow}
              windows={snapshot.query.data.windows}
            />
          ) : (
            <ProviderAllowanceUnavailable
              adapterAvailable={snapshot.adapterAvailable}
              error={snapshot.query.error}
              isLoading={snapshot.query.isPending && snapshot.adapterAvailable}
              productLabel={snapshot.productLabel}
            />
          )}

          <div className="space-y-2" data-testid="provider-allowance-agents">
            <div className="flex items-baseline justify-between gap-3">
              <h3 className="text-sm font-medium">Managed agents</h3>
              <span className="text-xs text-muted-foreground">
                {agents.length} {agents.length === 1 ? "agent" : "agents"}
              </span>
            </div>
            {agents.length > 0 ? (
              agents.map((agent) => (
                <AgentAllowanceRow
                  agent={agent}
                  key={agent.pubkey}
                  provider={snapshot.provider}
                  runtimes={runtimesQuery.data ?? []}
                  window={constrainingWindow}
                />
              ))
            ) : (
              <p className="rounded-xl bg-muted/25 px-4 py-3 text-sm text-muted-foreground">
                No managed agents yet.
              </p>
            )}
          </div>
        </Card>
      )}
    </section>
  );
}

function ProviderAllowanceSummary({
  matchingAgentCount,
  planType,
  productLabel,
  queryFailed,
  window,
  windows,
}: {
  matchingAgentCount: number;
  planType: string | null;
  productLabel: string;
  queryFailed: boolean;
  window: ProviderUsageWindow;
  windows: ProviderUsageWindow[];
}) {
  const level = providerAllowanceLevel(window.remainingPercent);
  const presentation = allowancePresentation[level];
  const planLabel = planType
    ? `${productLabel} ${planType.charAt(0).toUpperCase()}${planType.slice(1)}`
    : productLabel;

  return (
    <div className="space-y-3" data-testid="provider-allowance-summary">
      {queryFailed ? (
        <Alert variant="default">
          <AlertDescription>
            Last successful allowance shown; the latest refresh failed.
          </AlertDescription>
        </Alert>
      ) : null}
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <p className="text-sm font-semibold">{planLabel}</p>
          <p className="text-xs text-muted-foreground">
            Account-wide allowance shared by {matchingAgentCount}{" "}
            {matchingAgentCount === 1 ? "matching agent" : "matching agents"}
          </p>
        </div>
        <Badge variant={presentation.badge}>{presentation.label}</Badge>
      </div>
      <div>
        <div className="mb-1.5 flex items-baseline justify-between gap-3">
          <span className="text-xl font-semibold tabular-nums">
            {window.remainingPercent}% remaining
          </span>
          <span className="text-sm text-muted-foreground tabular-nums">
            {window.usedPercent}% used
          </span>
        </div>
        <Progress
          aria-label={`${productLabel}: ${window.remainingPercent}% remaining`}
          aria-valuetext={`${window.remainingPercent}% remaining for ${window.label}; resets ${formatUsageReset(window.resetsAt)}`}
          className={cn("h-2 bg-muted", presentation.progress)}
          value={window.remainingPercent}
        />
        <p className="mt-2 text-xs text-muted-foreground">
          {window.label} · Resets {formatUsageReset(window.resetsAt)}
        </p>
      </div>
      {windows.length > 1 ? (
        <dl className="grid gap-2 rounded-xl bg-muted/25 p-3 sm:grid-cols-2">
          {windows.map((candidate) => (
            <div className="min-w-0" key={candidate.id}>
              <dt className="truncate text-xs text-muted-foreground">
                {candidate.label}
              </dt>
              <dd className="text-sm font-medium tabular-nums">
                {candidate.remainingPercent}% remaining
              </dd>
              <dd className="text-xs text-muted-foreground">
                Resets {formatUsageReset(candidate.resetsAt)}
              </dd>
            </div>
          ))}
        </dl>
      ) : null}
    </div>
  );
}

function ProviderAllowanceUnavailable({
  adapterAvailable,
  error,
  isLoading,
  productLabel,
}: {
  adapterAvailable: boolean;
  error: unknown;
  isLoading: boolean;
  productLabel: string;
}) {
  if (isLoading) {
    return <ProviderAllowanceSkeleton compact />;
  }
  return (
    <Alert data-testid="provider-allowance-unavailable">
      <AlertDescription>
        <span className="font-medium">Unavailable from provider.</span>{" "}
        {adapterAvailable
          ? providerUsageErrorMessage(error)
          : `${productLabel} does not expose a supported local allowance source on this device.`}
      </AlertDescription>
    </Alert>
  );
}

function AgentAllowanceRow({
  agent,
  provider,
  runtimes,
  window,
}: {
  agent: ManagedAgent;
  provider: ProviderUsageId;
  runtimes: Parameters<typeof resolveAgentProviderUsage>[1];
  window: ProviderUsageWindow | null;
}) {
  const resolution = resolveAgentProviderUsage(agent, runtimes);
  const hasAllowance =
    resolution.providerUsageId === provider && window !== null;
  const detail = [
    resolution.runtimeLabel,
    agent.provider?.trim() || "Provider not reported",
    agent.model?.trim() || "Default model",
  ].join(" · ");

  return (
    <div
      className="flex flex-wrap items-center justify-between gap-3 rounded-xl bg-muted/25 px-4 py-3"
      data-testid={`provider-allowance-agent-${agent.pubkey}`}
    >
      <div className="min-w-0">
        <p className="truncate text-sm font-medium">{agent.name}</p>
        <p className="truncate text-xs text-muted-foreground">{detail}</p>
      </div>
      {hasAllowance ? (
        <div className="text-right">
          <p className="text-sm font-medium tabular-nums">
            {window.remainingPercent}% remaining
          </p>
          <p className="text-xs text-muted-foreground">
            Shared account allowance
          </p>
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">
          Unavailable from provider
        </p>
      )}
    </div>
  );
}

function ProviderAllowanceSkeleton({ compact = false }: { compact?: boolean }) {
  return (
    <div
      className={cn("space-y-3", !compact && "rounded-xl border p-6")}
      data-testid="provider-allowance-loading"
    >
      <Skeleton className="h-4 w-40" />
      <Skeleton className="h-8 w-56" />
      <Skeleton className="h-2 w-full" />
    </div>
  );
}
