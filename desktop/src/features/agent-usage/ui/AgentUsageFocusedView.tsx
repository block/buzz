import * as React from "react";
import { RefreshCw } from "lucide-react";

import { useAppShell } from "@/app/AppShellContext";
import type {
  AgentUsageModel,
  AgentUsageSeries,
} from "@/shared/api/tauriArchive";
import { Alert, AlertDescription } from "@/shared/ui/alert";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { Skeleton } from "@/shared/ui/skeleton";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import { useAgentUsageSeries } from "../hooks";
import {
  formatEstimatedCostUsd,
  formatTokenCountCompact,
  formatTokenCountExact,
  isPartialField,
  isUnknownField,
  parseTokenCount,
  sortModelsByKnownTotal,
  type UsageWindowDays,
} from "../lib/agentUsage";
import { AgentUsageDailyBars } from "./AgentUsageDailyBars";

/**
 * Per-agent Usage focused subview, rendered from the profile panel when
 * `view === 'usage'` (M4/A9/A13, frozen Rev 3 plan). Owns its own 7d/30d
 * selector and author-filtered query — independent of the Agents overview.
 *
 * A13 fail-closed: eligibility is ownership (`canViewUsage`) OR archived
 * evidence for a historical/deleted agent (`hasArchivedEvidence === true`).
 * A hand-authored `?profileView=usage` URL with neither falls back to the
 * summary view via `onIneligible` — but only once the query resolves, so a
 * still-loading owner-eligible or evidence-eligible agent is never bounced.
 */
export function AgentUsageFocusedView({
  agentPubkey,
  canViewUsage,
  onIneligible,
}: {
  agentPubkey: string;
  canViewUsage: boolean;
  onIneligible: () => void;
}) {
  const [days, setDays] = React.useState<UsageWindowDays>(7);
  const query = useAgentUsageSeries({ agentPubkey, days });
  const { onOpenSettings } = useAppShell();

  React.useEffect(() => {
    if (canViewUsage || !query.data) return;
    if (query.data.hasArchivedEvidence !== true) onIneligible();
  }, [canViewUsage, onIneligible, query.data]);

  return (
    <div className="space-y-4 pt-4" data-testid="agent-usage-focused-view">
      <Tabs
        onValueChange={(value) => setDays(value === "30" ? 30 : 7)}
        value={String(days)}
      >
        <TabsList>
          <TabsTrigger data-testid="agent-usage-focused-window-7" value="7">
            7d
          </TabsTrigger>
          <TabsTrigger data-testid="agent-usage-focused-window-30" value="30">
            30d
          </TabsTrigger>
        </TabsList>
      </Tabs>

      {query.isLoading ? (
        <AgentUsageFocusedSkeleton />
      ) : query.isError ? (
        <Alert data-testid="agent-usage-focused-error" variant="destructive">
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
        <AgentUsageFocusedContent
          days={days}
          onOpenSettings={onOpenSettings}
          series={query.data}
        />
      ) : null}
    </div>
  );
}

function AgentUsageFocusedSkeleton() {
  return (
    <Card className="space-y-4 p-6" data-testid="agent-usage-focused-skeleton">
      <Skeleton className="h-6 w-32" />
      <Skeleton className="h-16 w-full" />
      <Skeleton className="h-12 w-full" />
    </Card>
  );
}

function AgentUsageFocusedContent({
  days,
  onOpenSettings,
  series,
}: {
  days: UsageWindowDays;
  onOpenSettings: ((section: "local-archive") => void) | null;
  series: AgentUsageSeries;
}) {
  const agent = series.agents[0];
  const collectionOff = !series.collectionEnabled;
  const hasRetainedData = series.coverage.reportCount > 0;
  // Invalid-only: in-window invalid rows exist but none were bucketed (A5/A11).
  // Distinct from outside-window history — we have evidence in this window,
  // it just couldn't be counted. Must not be mislabeled as outside-window.
  const hasInvalidOnlyInWindow =
    agent === undefined &&
    series.collectionEnabled &&
    series.coverage.invalidReportCount > 0;
  const hasEvidenceOutsideWindow =
    agent === undefined &&
    !hasInvalidOnlyInWindow &&
    series.hasArchivedEvidence === true;

  if (
    !collectionOff &&
    agent === undefined &&
    !hasEvidenceOutsideWindow &&
    !hasInvalidOnlyInWindow
  ) {
    return (
      <p
        className="text-sm text-muted-foreground"
        data-testid="agent-usage-focused-empty"
      >
        No locally archived usage in the last {days} days. Usage appears after
        this agent completes a usage-reporting turn.
      </p>
    );
  }

  return (
    <div className="space-y-4">
      {collectionOff ? (
        <Alert data-testid="agent-usage-focused-collection-off">
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

      {agent ? (
        <AgentUsageFocusedTotals agent={agent} coverage={series.coverage} />
      ) : hasEvidenceOutsideWindow ? (
        <p
          className="text-sm text-muted-foreground"
          data-testid="agent-usage-focused-outside-window"
        >
          No locally archived usage in the last {days} days, but this agent has
          reported usage previously. Try the 30-day window.
        </p>
      ) : hasInvalidOnlyInWindow ? (
        <p
          className="text-sm text-muted-foreground"
          data-testid="agent-usage-focused-invalid-only"
        >
          Usage was collected in the last {days} days but could not be counted —
          reports with unreadable timestamps or missing session totals are
          excluded and are not assigned to any day.
        </p>
      ) : null}
    </div>
  );
}

function AgentUsageFocusedTotals({
  agent,
  coverage,
}: {
  agent: AgentUsageSeries["agents"][number];
  coverage: AgentUsageSeries["coverage"];
}) {
  const { estimatedCostUsd, inputTokens, outputTokens, totalTokens } =
    agent.usage;
  const models = sortModelsByKnownTotal(agent.models);
  const explainPartial =
    agent.hasUnknownUsage || coverage.invalidReportCount > 0;

  return (
    <Card className="space-y-4 p-6" data-testid="agent-usage-focused-totals">
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <TokenStat field={totalTokens} label="Total tokens" />
        <TokenStat field={inputTokens} label="Input tokens" />
        <TokenStat field={outputTokens} label="Output tokens" />
        <UsageStat
          display={
            estimatedCostUsd.value !== null
              ? `Est. ${formatEstimatedCostUsd(estimatedCostUsd.value)}`
              : null
          }
          isPartial={isPartialField(estimatedCostUsd)}
          label="Estimated cost"
        />
      </div>

      {agent.buckets.length > 0 ? (
        <div
          className="space-y-2 border-t border-border pt-4"
          data-testid="agent-usage-focused-daily-bars"
        >
          <h3 className="text-sm font-medium text-foreground">Daily usage</h3>
          <AgentUsageDailyBars buckets={agent.buckets} />
        </div>
      ) : null}

      {models.length > 0 ? (
        <div
          className="space-y-2 border-t border-border pt-4"
          data-testid="agent-usage-focused-models"
        >
          <h3 className="text-sm font-medium text-foreground">By model</h3>
          {models.map((model) => (
            <div
              className="flex items-center justify-between gap-3 text-sm"
              key={model.model ?? "unknown"}
            >
              <span className="min-w-0 truncate text-muted-foreground">
                {model.model ?? "Unknown model"}
              </span>
              <span className="shrink-0 font-medium text-foreground">
                {isUnknownField(model.usage.totalTokens)
                  ? formatModelIndependentFields(model)
                  : formatTokenCountExact(
                      parseTokenCount(model.usage.totalTokens.value) ?? 0n,
                    )}
                {isPartialField(model.usage.totalTokens) ||
                isModelIoPartial(model) ? (
                  <Badge className="ml-2" variant="outline">
                    Partial
                  </Badge>
                ) : null}
              </span>
            </div>
          ))}
        </div>
      ) : null}

      <div
        className="space-y-1 border-t border-border pt-4 text-xs text-muted-foreground"
        data-testid="agent-usage-focused-coverage"
      >
        <p>
          {agent.reportCount} reported turn{agent.reportCount === 1 ? "" : "s"}
          {" · "}
          {formatCoverageRange(coverage)}
        </p>
        {explainPartial ? (
          <p data-testid="agent-usage-focused-partial-explanation">
            Some usage could not be counted: reports with an unreadable
            timestamp or a cumulative total missing its session are excluded,
            and unknown intervals are omitted rather than shown as zero.
          </p>
        ) : null}
      </div>
    </Card>
  );
}

function TokenStat({
  field,
  label,
}: {
  field: { value: string | null; incomplete: boolean };
  label: string;
}) {
  const parsed = parseTokenCount(field.value);
  return (
    <UsageStat
      display={parsed !== null ? formatTokenCountExact(parsed) : null}
      isPartial={isPartialField(field)}
      label={label}
    />
  );
}

function UsageStat({
  display,
  isPartial,
  label,
}: {
  display: string | null;
  isPartial: boolean;
  label: string;
}) {
  return (
    <div className="space-y-1">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className="text-lg font-semibold text-foreground">{display ?? "—"}</p>
      {isPartial ? <Badge variant="outline">Partial</Badge> : null}
    </div>
  );
}

function formatCoverageDate(unixSeconds: number | null): string {
  if (unixSeconds === null) return "unknown";
  return new Date(unixSeconds * 1000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

/**
 * Human-readable coverage range for the focused view's footer, from the
 * exact first/last reported timestamps the backend already computes
 * (plan:329's "coverage dates"). `null` on either end means no reported row
 * fell in this window (the caller only renders this once `agent` exists,
 * so both are actually set in practice, but the fallback stays honest).
 */
function formatCoverageRange(coverage: AgentUsageSeries["coverage"]): string {
  const { firstReportedAt, lastReportedAt } = coverage;
  if (firstReportedAt === null || lastReportedAt === null) {
    return "coverage unknown";
  }
  if (firstReportedAt === lastReportedAt) {
    return `reported ${formatCoverageDate(firstReportedAt)}`;
  }
  return `${formatCoverageDate(firstReportedAt)} – ${formatCoverageDate(lastReportedAt)}`;
}

/**
 * Render known model I/O fields when the model total is unknown — never
 * collapses to "No usage reported" when input or output is actually known
 * (A2 per-field completeness). Mirrors `formatIndependentFields` in the
 * overview row.
 */
function formatModelIndependentFields(model: AgentUsageModel): string {
  const input = parseTokenCount(model.usage.inputTokens.value);
  const output = parseTokenCount(model.usage.outputTokens.value);
  if (input !== null || output !== null) {
    const parts: string[] = [];
    if (input !== null) parts.push(`in ${formatTokenCountCompact(input)}`);
    if (output !== null) parts.push(`out ${formatTokenCountCompact(output)}`);
    return parts.join(" · ");
  }
  return "No usage reported";
}

/**
 * True when a model has no known total but its displayed I/O fields carry
 * incomplete truth — so the Partial badge must still appear (A2).
 */
function isModelIoPartial(model: AgentUsageModel): boolean {
  return (
    isUnknownField(model.usage.totalTokens) &&
    (isPartialField(model.usage.inputTokens) ||
      isPartialField(model.usage.outputTokens))
  );
}
