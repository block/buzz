import * as React from "react";
import { Activity, BarChart3, Loader2 } from "lucide-react";

import { useOpenAgentActivity } from "@/features/agents/useOpenAgentActivity";
import { useAgentUsageSeries } from "@/features/agents/useAgentUsageSeries";
import { Button } from "@/shared/ui/button";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/shared/ui/sheet";
import { cn } from "@/shared/lib/cn";

function formatTokenField(value: string | null, incomplete: boolean): string {
  if (value === null) {
    return incomplete ? "unknown" : "—";
  }
  try {
    const n = BigInt(value);
    if (n >= 1_000_000n) {
      return `${(Number(n) / 1_000_000).toFixed(1)}M`;
    }
    if (n >= 1_000n) {
      return `${(Number(n) / 1_000).toFixed(1)}k`;
    }
    return n.toString();
  } catch {
    return value;
  }
}

function formatCost(value: number | null, incomplete: boolean): string {
  if (value === null) {
    return incomplete ? "unknown" : "—";
  }
  return `$${value.toFixed(value < 0.01 ? 4 : 2)}`;
}

function bucketLabel(startSec: number): string {
  const d = new Date(startSec * 1000);
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export type AgentUsageActivityPanelProps = {
  agentPubkey: string;
  agentName?: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Optional channel to deep-link when opening the activity pane. */
  channelId?: string | null;
};

/**
 * LMStudio-style Usage & activity drawer: token spend series from the local
 * metric archive (`get_agent_usage_series`) plus a deep-link into the existing
 * agent activity pane (observer thinking / tool calls).
 */
export function AgentUsageActivityPanel({
  agentPubkey,
  agentName,
  open,
  onOpenChange,
  channelId = null,
}: AgentUsageActivityPanelProps) {
  const usageQuery = useAgentUsageSeries(agentPubkey, {
    dayCount: 7,
    enabled: open,
  });
  const { openAgentActivity, canOpenAgentActivity } = useOpenAgentActivity();

  const series = usageQuery.data;
  const agentRow = series?.agents.find(
    (row) => row.agentPubkey.toLowerCase() === agentPubkey.toLowerCase(),
  );
  const usage = agentRow?.usage ?? series?.agents[0]?.usage;
  const buckets = agentRow?.buckets ?? series?.buckets ?? [];
  const collectionEnabled = series?.collectionEnabled ?? true;
  const maxTotal = React.useMemo(() => {
    let max = 0n;
    for (const bucket of buckets) {
      const raw = bucket.usage.totalTokens.value;
      if (raw === null) continue;
      try {
        const n = BigInt(raw);
        if (n > max) max = n;
      } catch {
        /* ignore */
      }
    }
    return max > 0n ? max : 1n;
  }, [buckets]);

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        className="flex w-full flex-col gap-0 sm:max-w-md"
        data-testid={`agent-usage-activity-panel-${agentPubkey}`}
        side="right"
      >
        <SheetHeader className="border-b border-border/60 pb-4 text-left">
          <SheetTitle className="flex items-center gap-2">
            <BarChart3 className="h-4 w-4 text-muted-foreground" />
            Usage &amp; activity
          </SheetTitle>
          <SheetDescription>
            {agentName
              ? `Token spend and session activity for ${agentName}.`
              : "Token spend and session activity for this agent."}
          </SheetDescription>
        </SheetHeader>

        <div className="flex-1 space-y-6 overflow-y-auto py-4">
          {!collectionEnabled ? (
            <div
              className="rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-800 dark:text-amber-200"
              data-testid="agent-usage-archive-nudge"
            >
              Local metric archive is off. Turn on &ldquo;Archive agent turn
              metrics&rdquo; in Settings → Local archive to keep a personal
              usage history. Kind 44200 events may still exist on the relay.
            </div>
          ) : null}

          <section className="space-y-3">
            <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
              Last 7 days
            </h3>
            {usageQuery.isLoading ? (
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <Loader2 className="h-4 w-4 animate-spin" />
                Loading usage…
              </div>
            ) : usageQuery.isError ? (
              <p className="text-sm text-destructive">
                Couldn&apos;t load usage series.
              </p>
            ) : (
              <>
                <div className="grid grid-cols-3 gap-2">
                  <StatCard
                    label="Input"
                    value={formatTokenField(
                      usage?.inputTokens.value ?? null,
                      usage?.inputTokens.incomplete ?? false,
                    )}
                  />
                  <StatCard
                    label="Output"
                    value={formatTokenField(
                      usage?.outputTokens.value ?? null,
                      usage?.outputTokens.incomplete ?? false,
                    )}
                  />
                  <StatCard
                    label="Est. cost"
                    value={formatCost(
                      usage?.estimatedCostUsd.value ?? null,
                      usage?.estimatedCostUsd.incomplete ?? false,
                    )}
                  />
                </div>

                <div
                  className="flex h-24 items-end gap-1 rounded-lg border border-border/60 bg-muted/20 px-2 py-2"
                  data-testid="agent-usage-bucket-chart"
                >
                  {buckets.length === 0 ? (
                    <p className="m-auto text-xs text-muted-foreground">
                      No usage reports in this window.
                    </p>
                  ) : (
                    buckets.map((bucket) => {
                      let heightPct = 4;
                      const raw = bucket.usage.totalTokens.value;
                      if (raw !== null) {
                        try {
                          const n = BigInt(raw);
                          heightPct = Math.max(
                            4,
                            Number((n * 100n) / maxTotal),
                          );
                        } catch {
                          heightPct = 4;
                        }
                      }
                      return (
                        <div
                          className="flex min-w-0 flex-1 flex-col items-center justify-end gap-1"
                          key={`${bucket.start}-${bucket.end}`}
                          title={`${bucketLabel(bucket.start)}: ${formatTokenField(raw, bucket.usage.totalTokens.incomplete)} tokens`}
                        >
                          <div
                            className={cn(
                              "w-full rounded-sm bg-primary/70",
                              bucket.reportCount === 0 && "bg-muted-foreground/20",
                            )}
                            style={{ height: `${heightPct}%` }}
                          />
                          <span className="truncate text-3xs text-muted-foreground">
                            {bucketLabel(bucket.start)}
                          </span>
                        </div>
                      );
                    })
                  )}
                </div>

                {series?.coverage.hasUnknownUsage || agentRow?.hasUnknownUsage ? (
                  <p className="text-2xs text-muted-foreground">
                    Some turns reported incomplete token counts — totals may
                    understate real usage.
                  </p>
                ) : null}
              </>
            )}
          </section>

          <section className="space-y-3">
            <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
              Activity
            </h3>
            <p className="text-sm text-muted-foreground">
              Open the live activity pane for thinking, tool calls, and session
              history (from the relay and your local observer archive).
            </p>
            <Button
              className="w-full justify-start gap-2"
              data-testid={`agent-usage-open-activity-${agentPubkey}`}
              disabled={!canOpenAgentActivity(agentPubkey)}
              onClick={() => {
                const opened = openAgentActivity(agentPubkey, {
                  channelId,
                });
                if (opened) {
                  onOpenChange(false);
                }
              }}
              type="button"
              variant="outline"
            >
              <Activity className="h-4 w-4" />
              View activity
            </Button>
          </section>
        </div>
      </SheetContent>
    </Sheet>
  );
}

function StatCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-border/60 bg-background px-2.5 py-2">
      <p className="text-2xs font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </p>
      <p className="mt-0.5 text-sm font-semibold tabular-nums text-foreground">
        {value}
      </p>
    </div>
  );
}

/**
 * Button that opens {@link AgentUsageActivityPanel}. Use on local agent rows
 * and community bot surfaces alongside (or instead of) plain View activity.
 */
export function UsageAndActivityButton({
  agentPubkey,
  agentName,
  channelId = null,
  className,
  size = "sm",
  variant = "outline",
  label = "Usage & activity",
  testId,
}: {
  agentPubkey: string;
  agentName?: string | null;
  channelId?: string | null;
  className?: string;
  size?: "sm" | "default" | "lg" | "icon";
  variant?: "outline" | "ghost" | "secondary" | "default";
  label?: string;
  testId?: string;
}) {
  const [open, setOpen] = React.useState(false);
  return (
    <>
      <Button
        className={className}
        data-testid={testId ?? `usage-activity-btn-${agentPubkey}`}
        onClick={(event) => {
          event.stopPropagation();
          setOpen(true);
        }}
        size={size}
        type="button"
        variant={variant}
      >
        <BarChart3 className="h-4 w-4" />
        {label}
      </Button>
      <AgentUsageActivityPanel
        agentName={agentName}
        agentPubkey={agentPubkey}
        channelId={channelId}
        onOpenChange={setOpen}
        open={open}
      />
    </>
  );
}
