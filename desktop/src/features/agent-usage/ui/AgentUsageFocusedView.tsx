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
import { useAgentUsageSeries } from "../hooks";
import {
  DEFAULT_USAGE_RANGE,
  deriveDisplayTotal,
  describeRange,
  formatCoverageDate,
  formatEstimatedCostUsd,
  formatTokenCountCompact,
  formatTokenCountExact,
  isPartialField,
  isUnknownField,
  parseTokenCount,
  sortModelsByDisplayTotal,
  type DisplayTotal,
  type UsageRange,
} from "../lib/agentUsage";
import { AgentUsageDailyBars } from "./AgentUsageDailyBars";
import { AgentUsageRangeTabs } from "./AgentUsageRangeTabs";

/**
 * Per-agent Usage focused subview, rendered from the profile panel when
 * `view === 'usage'` (M4/A9/A13, frozen Rev 3 plan). Owns its own window
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
  const [range, setRange] = React.useState<UsageRange>(DEFAULT_USAGE_RANGE);
  const query = useAgentUsageSeries({ agentPubkey, range });
  const { onOpenSettings } = useAppShell();

  React.useEffect(() => {
    if (canViewUsage || !query.data) return;
    if (query.data.hasArchivedEvidence !== true) onIneligible();
  }, [canViewUsage, onIneligible, query.data]);

  return (
    <div className="space-y-4 pt-4" data-testid="agent-usage-focused-view">
      <AgentUsageRangeTabs
        onRangeChange={setRange}
        range={range}
        testIdPrefix="agent-usage-focused-window"
      />

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
          onOpenSettings={onOpenSettings}
          range={range}
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
  onOpenSettings,
  range,
  series,
}: {
  onOpenSettings: ((section: "local-archive") => void) | null;
  range: UsageRange;
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
        No locally archived usage in {describeRange(range)}. Usage appears after
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
          No locally archived usage in {describeRange(range)}, but this agent
          has reported usage previously. Try a wider window.
        </p>
      ) : hasInvalidOnlyInWindow ? (
        <p
          className="text-sm text-muted-foreground"
          data-testid="agent-usage-focused-invalid-only"
        >
          Usage was collected in {describeRange(range)} but could not be counted
          — reports with unreadable timestamps or missing session totals are
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
  const { estimatedCostUsd, inputTokens, outputTokens } = agent.usage;
  const models = sortModelsByDisplayTotal(agent.models);
  // Each caveat sentence is gated only on the condition that proves it:
  //   - unknown-intervals sentence: direct i/o incompleteness — true when at
  //     least one input or output field is known but flagged incomplete. This
  //     is the condition the copy claims ("input/output usage could not be
  //     counted"). `hasUnknownUsage` is NOT used here because it ORs total
  //     and cost incompleteness too, which cannot prove an i/o interval claim.
  //   - invalid-reports sentence: `coverage.invalidReportCount > 0` — true
  //     when rows were excluded from buckets due to bad timestamps or missing
  //     session cumulative totals.
  // We do NOT trigger on totalTokens.value being null — that's the permanent
  // state for all real publishers today, not a data quality problem.
  const showUnknownIntervalsCaveat =
    isPartialField(inputTokens) || isPartialField(outputTokens);
  const showInvalidReportsCaveat = coverage.invalidReportCount > 0;

  // Display total for the Total tokens stat.
  const displayTotal = deriveDisplayTotal(agent.usage);

  return (
    <Card className="space-y-4 p-6" data-testid="agent-usage-focused-totals">
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <ApproxTokenStat displayTotal={displayTotal} label="Total tokens" />
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
              key={`${model.harness ?? ""}:${model.model ?? "unknown"}`}
            >
              <span className="min-w-0 truncate text-muted-foreground">
                {model.model ?? "Unknown model"}
                {model.harness !== null ? (
                  <span
                    className="ml-1.5 text-xs text-muted-foreground/70"
                    data-testid="agent-usage-model-harness-label"
                  >
                    {model.harness}
                  </span>
                ) : null}
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
        {showUnknownIntervalsCaveat ? (
          <p data-testid="agent-usage-focused-unknown-intervals-caveat">
            Some input/output usage could not be counted and is omitted rather
            than shown as zero.
          </p>
        ) : null}
        {showInvalidReportsCaveat ? (
          <p data-testid="agent-usage-focused-invalid-reports-caveat">
            {coverage.invalidReportCount === 1
              ? "1 report"
              : `${coverage.invalidReportCount} reports`}{" "}
            excluded: reports with an unreadable timestamp or a cumulative total
            missing its session are not assigned to any day.
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

/**
 * Total-tokens stat that falls back to an `≈` approximation (in+out) when
 * the genuine total is unavailable. The `≈` prefix keeps the approximation
 * honest without hiding that real token activity was counted.
 */
function ApproxTokenStat({
  displayTotal,
  label,
}: {
  displayTotal: DisplayTotal;
  label: string;
}) {
  const display =
    displayTotal.kind === "exact"
      ? formatTokenCountExact(displayTotal.value)
      : displayTotal.kind === "approximate"
        ? `≈ ${formatTokenCountExact(displayTotal.value)}`
        : null;
  return (
    <UsageStat
      display={display}
      isPartial={displayTotal.partial}
      label={label}
      testId="agent-usage-focused-total-value"
    />
  );
}

function UsageStat({
  display,
  isPartial,
  label,
  testId,
}: {
  display: string | null;
  isPartial: boolean;
  label: string;
  testId?: string;
}) {
  return (
    <div className="space-y-1">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className="text-lg font-semibold text-foreground" data-testid={testId}>
        {display ?? "—"}
      </p>
      {isPartial ? <Badge variant="outline">Partial</Badge> : null}
    </div>
  );
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
