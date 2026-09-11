import * as React from "react";
import { RefreshCw } from "lucide-react";

import { useAppShell } from "@/app/AppShellContext";
import type { ProfilePanelOpenOptions } from "@/shared/context/ProfilePanelContext";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { AgentUsage, AgentUsageSeries } from "@/shared/api/tauriArchive";
import { Alert, AlertDescription } from "@/shared/ui/alert";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { SectionHeader } from "@/shared/ui/PageHeader";
import { Progress } from "@/shared/ui/progress";
import { Skeleton } from "@/shared/ui/skeleton";
import { useAgentUsageSeries } from "../hooks";
import {
  bigintRatio,
  DEFAULT_USAGE_RANGE,
  deriveDisplayTotal,
  describeRange,
  formatCoverageDate,
  formatTokenCountCompact,
  sortAgentsByDisplayTotal,
  sumKnownBucketTotals,
  type UsageRange,
} from "../lib/agentUsage";
import { AgentUsageDailyBars } from "./AgentUsageDailyBars";
import { AgentUsageRangeTabs } from "./AgentUsageRangeTabs";

/**
 * Compact "Usage" section on the Agents page: local NIP-AM usage totals for
 * the selected window (1d/7d/30d preset or a custom date range), broken down
 * per agent, with a click-through to the per-agent focused view in the
 * profile panel (M4/A9/A13, frozen Rev 3 plan).
 */
export function AgentUsageSection({
  onOpenAgentProfile,
}: {
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
}) {
  const [range, setRange] = React.useState<UsageRange>(DEFAULT_USAGE_RANGE);
  const query = useAgentUsageSeries({ range });
  const { onOpenSettings } = useAppShell();

  const agents = React.useMemo(
    () => sortAgentsByDisplayTotal(query.data?.agents ?? []),
    [query.data?.agents],
  );
  const pubkeys = React.useMemo(
    () => agents.map((agent) => agent.agentPubkey),
    [agents],
  );
  const usersBatchQuery = useUsersBatchQuery(pubkeys, {
    enabled: pubkeys.length > 0,
  });

  return (
    <section className="relative space-y-4" data-testid="agents-usage-section">
      <SectionHeader
        action={
          <AgentUsageRangeTabs
            onRangeChange={setRange}
            range={range}
            testIdPrefix="agent-usage-window"
          />
        }
        description="Locally archived, agent-reported usage."
        title="Usage"
      />

      {query.isLoading ? (
        <AgentUsageSkeleton />
      ) : query.isError ? (
        <Alert data-testid="agent-usage-error" variant="destructive">
          <AlertDescription className="flex items-center justify-between gap-3">
            <span>Couldn't load usage data.</span>
            <Button
              onClick={() => void query.refetch()}
              size="sm"
              variant="outline"
            >
              <RefreshCw className="h-3.5 w-3.5" />
              Retry
            </Button>
          </AlertDescription>
        </Alert>
      ) : query.data ? (
        <AgentUsageCard
          agents={agents}
          onOpenAgentProfile={onOpenAgentProfile}
          range={range}
          onOpenSettings={onOpenSettings}
          profiles={usersBatchQuery.data?.profiles}
          series={query.data}
        />
      ) : null}
    </section>
  );
}

function AgentUsageSkeleton() {
  return (
    <Card className="space-y-4 p-6" data-testid="agent-usage-skeleton">
      <Skeleton className="h-4 w-40" />
      <Skeleton className="h-16 w-full" />
      <Skeleton className="h-12 w-full" />
      <Skeleton className="h-12 w-full" />
    </Card>
  );
}

function AgentUsageCard({
  agents,
  onOpenAgentProfile,
  onOpenSettings,
  profiles,
  range,
  series,
}: {
  agents: AgentUsage[];
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  onOpenSettings: ((section: "local-archive") => void) | null;
  profiles: UserProfileLookup | undefined;
  range: UsageRange;
  series: AgentUsageSeries;
}) {
  const hasRows = agents.length > 0;
  const collectionOff = !series.collectionEnabled;
  const hasRetainedData = series.coverage.reportCount > 0;
  // True when the window has in-window invalid rows but no valid/bucketed rows.
  // These rows are correctly excluded from buckets (A5/A11) but the window is
  // not empty — coverage.hasUnknownUsage reflects this via the F1 roll-up.
  const hasInvalidOnlyInWindow =
    !hasRows &&
    series.collectionEnabled &&
    series.coverage.invalidReportCount > 0;

  // Relative bars are decorative (aria-hidden, per plan) — scale each agent's
  // display total (exact or approximate) against the largest such value in the
  // current window so the sorted-by-display-total list also reads as a bar chart.
  const maxDisplayValue = React.useMemo(
    () =>
      agents.reduce<bigint>((max, agent) => {
        const dt = deriveDisplayTotal(agent.usage);
        return dt.value !== null && dt.value > max ? dt.value : max;
      }, 0n),
    [agents],
  );

  const overallTotal = React.useMemo(
    () => sumKnownBucketTotals(series.buckets),
    [series.buckets],
  );

  return (
    <Card className="space-y-4 p-6" data-testid="agent-usage-card">
      {series.buckets.length > 0 ? (
        <div className="space-y-2" data-testid="agent-usage-overall-bars">
          <div className="flex items-baseline justify-between gap-2">
            <h3 className="text-sm font-medium text-foreground">Daily usage</h3>
            <span
              className="text-sm text-muted-foreground"
              data-testid="agent-usage-header-value"
            >
              {overallTotal.kind === "exact"
                ? `${formatTokenCountCompact(overallTotal.value)} tokens`
                : overallTotal.kind === "approximate"
                  ? `≈ ${formatTokenCountCompact(overallTotal.value)} tokens`
                  : hasInvalidOnlyInWindow
                    ? "Usage uncountable"
                    : "No usage reported"}
              {(overallTotal.kind !== "unknown" && overallTotal.partial) ||
              hasInvalidOnlyInWindow ? (
                <Badge className="ml-2" variant="outline">
                  Partial
                </Badge>
              ) : null}
            </span>
          </div>
          <AgentUsageDailyBars buckets={series.buckets} />
        </div>
      ) : null}

      {collectionOff ? (
        <Alert data-testid="agent-usage-collection-off">
          <AlertDescription className="flex flex-wrap items-center justify-between gap-2">
            <span>
              {hasRetainedData
                ? `Collection off · data through ${formatCoverageDate(
                    series.coverage.lastArchivedAt,
                  )}`
                : "Local usage collection is off."}
            </span>
            <Button
              onClick={() => onOpenSettings?.("local-archive")}
              size="sm"
              variant="outline"
            >
              Open Local Archive settings
            </Button>
          </AlertDescription>
        </Alert>
      ) : null}

      {hasRows ? (
        <div className="space-y-3">
          {agents.map((agent) => (
            <AgentUsageRow
              agent={agent}
              key={agent.agentPubkey}
              label={resolveUserLabel({ profiles, pubkey: agent.agentPubkey })}
              maxDisplayValue={maxDisplayValue}
              onOpenAgentProfile={onOpenAgentProfile}
              profileAvatarUrl={
                profiles?.[agent.agentPubkey]?.avatarUrl ?? null
              }
              range={range}
            />
          ))}
        </div>
      ) : (
        <p
          className="text-sm text-muted-foreground"
          data-testid="agent-usage-empty"
        >
          {collectionOff
            ? "Turn on collection to start tracking agent usage."
            : hasInvalidOnlyInWindow
              ? `Usage was collected in ${describeRange(range)} but could not be counted — reports with unreadable timestamps or missing session totals are excluded.`
              : `No locally archived usage in ${describeRange(range)}. Usage appears after an agent completes a usage-reporting turn.`}
        </p>
      )}
    </Card>
  );
}

function AgentUsageRow({
  agent,
  label,
  maxDisplayValue,
  onOpenAgentProfile,
  profileAvatarUrl,
  range,
}: {
  agent: AgentUsage;
  label: string;
  maxDisplayValue: bigint;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  profileAvatarUrl: string | null;
  range: UsageRange;
}) {
  const dt = deriveDisplayTotal(agent.usage);

  const trailing =
    dt.kind === "exact"
      ? formatTokenCountCompact(dt.value)
      : dt.kind === "approximate"
        ? `≈ ${formatTokenCountCompact(dt.value)}`
        : "No usage reported";

  return (
    <button
      aria-label={`Open ${label} usage for ${describeRange(range)}`}
      className="flex w-full items-center gap-3 rounded-2xl bg-muted/20 px-4 py-3 text-left transition-colors hover:bg-muted/40"
      data-testid={`agent-usage-row-${agent.agentPubkey}`}
      onClick={() => onOpenAgentProfile(agent.agentPubkey, { view: "usage" })}
      type="button"
    >
      <ProfileAvatar
        avatarUrl={profileAvatarUrl}
        className="h-9 w-9"
        label={label}
      />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-foreground">
          {label}
        </span>
        {dt.kind !== "unknown" ? (
          <Progress
            aria-hidden="true"
            className="mt-1.5 h-1.5"
            value={
              maxDisplayValue > 0n
                ? bigintRatio(dt.value, maxDisplayValue) * 100
                : null
            }
          />
        ) : null}
      </span>
      <span className="flex shrink-0 items-center gap-2 text-sm text-muted-foreground">
        {dt.partial ? <Badge variant="outline">Partial</Badge> : null}
        {trailing}
      </span>
    </button>
  );
}
