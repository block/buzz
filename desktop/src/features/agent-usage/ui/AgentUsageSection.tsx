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
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import { useAgentUsageSeries } from "../hooks";
import {
  bigintRatio,
  formatTokenCountCompact,
  isPartialField,
  isUnknownField,
  parseTokenCount,
  sortAgentsByKnownTotal,
  sumKnownBucketTotals,
  type UsageWindowDays,
} from "../lib/agentUsage";
import { AgentUsageDailyBars } from "./AgentUsageDailyBars";

/**
 * Compact "Usage" section on the Agents page: local NIP-AM usage totals for
 * the last 7 or 30 days, broken down per agent, with a click-through to the
 * per-agent focused view in the profile panel (M4/A9/A13, frozen Rev 3 plan).
 */
export function AgentUsageSection({
  onOpenAgentProfile,
}: {
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
}) {
  const [days, setDays] = React.useState<UsageWindowDays>(7);
  const query = useAgentUsageSeries({ days });
  const { onOpenSettings } = useAppShell();

  const agents = React.useMemo(
    () => sortAgentsByKnownTotal(query.data?.agents ?? []),
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
          <Tabs
            onValueChange={(value) => setDays(value === "30" ? 30 : 7)}
            value={String(days)}
          >
            <TabsList>
              <TabsTrigger data-testid="agent-usage-window-7" value="7">
                7d
              </TabsTrigger>
              <TabsTrigger data-testid="agent-usage-window-30" value="30">
                30d
              </TabsTrigger>
            </TabsList>
          </Tabs>
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
          days={days}
          onOpenAgentProfile={onOpenAgentProfile}
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
  days,
  onOpenAgentProfile,
  onOpenSettings,
  profiles,
  series,
}: {
  agents: AgentUsage[];
  days: UsageWindowDays;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  onOpenSettings: ((section: "local-archive") => void) | null;
  profiles: UserProfileLookup | undefined;
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
  // known total against the largest known total in the current window so the
  // sorted-by-total list also reads as a bar chart.
  const maxKnownTotal = React.useMemo(
    () =>
      agents.reduce<bigint>((max, agent) => {
        const total = parseTokenCount(agent.usage.totalTokens.value);
        return total !== null && total > max ? total : max;
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
            <span className="text-sm text-muted-foreground">
              {overallTotal.knownTotal !== null
                ? `${formatTokenCountCompact(overallTotal.knownTotal)} tokens`
                : hasInvalidOnlyInWindow
                  ? "Usage uncountable"
                  : "No usage reported"}
              {overallTotal.partial || hasInvalidOnlyInWindow ? (
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
              days={days}
              key={agent.agentPubkey}
              label={resolveUserLabel({ profiles, pubkey: agent.agentPubkey })}
              maxKnownTotal={maxKnownTotal}
              onOpenAgentProfile={onOpenAgentProfile}
              profileAvatarUrl={
                profiles?.[agent.agentPubkey]?.avatarUrl ?? null
              }
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
              ? `Usage was collected in the last ${days} days but could not be counted — reports with unreadable timestamps or missing session totals are excluded.`
              : `No locally archived usage in the last ${days} days. Usage appears after an agent completes a usage-reporting turn.`}
        </p>
      )}
    </Card>
  );
}

function formatCoverageDate(unixSeconds: number | null): string {
  if (unixSeconds === null) return "unknown";
  return new Date(unixSeconds * 1000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

function AgentUsageRow({
  agent,
  days,
  label,
  maxKnownTotal,
  onOpenAgentProfile,
  profileAvatarUrl,
}: {
  agent: AgentUsage;
  days: UsageWindowDays;
  label: string;
  maxKnownTotal: bigint;
  onOpenAgentProfile: (
    pubkey: string,
    options?: ProfilePanelOpenOptions,
  ) => void;
  profileAvatarUrl: string | null;
}) {
  const total = agent.usage.totalTokens;
  const knownTotal = parseTokenCount(total.value);
  const partial = isPartialField(total);
  const unknown = isUnknownField(total);

  // When total is null (unknown), check whether any displayed I/O field is
  // incomplete — per-field partial truth must be preserved at every seam (A2).
  const ioPartial =
    knownTotal === null &&
    (isPartialField(agent.usage.inputTokens) ||
      isPartialField(agent.usage.outputTokens));

  const trailing =
    knownTotal !== null
      ? formatTokenCountCompact(knownTotal)
      : formatIndependentFields(agent);

  return (
    <button
      aria-label={`Open ${label} usage for the last ${days} days`}
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
        {!unknown ? (
          <Progress
            aria-hidden="true"
            className="mt-1.5 h-1.5"
            value={
              knownTotal !== null && maxKnownTotal > 0n
                ? bigintRatio(knownTotal, maxKnownTotal) * 100
                : null
            }
          />
        ) : null}
      </span>
      <span className="flex shrink-0 items-center gap-2 text-sm text-muted-foreground">
        {partial || ioPartial ? <Badge variant="outline">Partial</Badge> : null}
        {trailing}
      </span>
    </button>
  );
}

function formatIndependentFields(agent: AgentUsage): string {
  const input = parseTokenCount(agent.usage.inputTokens.value);
  const output = parseTokenCount(agent.usage.outputTokens.value);
  if (input !== null || output !== null) {
    const parts: string[] = [];
    if (input !== null) parts.push(`in ${formatTokenCountCompact(input)}`);
    if (output !== null) parts.push(`out ${formatTokenCountCompact(output)}`);
    return parts.join(" · ");
  }
  return "No usage reported";
}
