import { AlertCircle, Bot, Clock3 } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Progress } from "@/shared/ui/progress";
import type { AgentStatusViewModel, AgentUsageWindowSnapshot } from "./types";
import { usePermanentAgentStatuses } from "./useAgentStatusAdapter";

function formatPercent(value: number): string {
  return `${Math.round(value)}%`;
}

function formatTokenCount(value: bigint | null): string {
  if (value === null) return "—";
  return new Intl.NumberFormat(undefined, {
    notation: value >= 10_000n ? "compact" : "standard",
    maximumFractionDigits: 1,
  }).format(value);
}

export function formatDataAge(ageSeconds: number | null): string | null {
  if (ageSeconds === null) return null;
  if (ageSeconds < 60) return "Updated just now";
  if (ageSeconds < 3_600) return `Updated ${Math.floor(ageSeconds / 60)}m ago`;
  if (ageSeconds < 86_400)
    return `Updated ${Math.floor(ageSeconds / 3_600)}h ago`;
  return `Updated ${Math.floor(ageSeconds / 86_400)}d ago`;
}

export function formatReset(
  resetAt: number | null | undefined,
  nowSeconds: number,
): string | null {
  if (resetAt === null || resetAt === undefined) return null;
  const remaining = Math.max(0, resetAt - nowSeconds);
  if (remaining < 60) return "Resets now";
  if (remaining < 3_600) return `Resets in ${Math.ceil(remaining / 60)}m`;
  if (remaining < 86_400) return `Resets in ${Math.ceil(remaining / 3_600)}h`;
  return `Resets in ${Math.ceil(remaining / 86_400)}d`;
}

function MetricBar({
  label,
  percent,
  detail,
  className,
}: {
  label: string;
  percent: number;
  detail?: string | null;
  className?: string;
}) {
  return (
    <div className={cn("space-y-1", className)}>
      <div className="flex min-w-0 items-baseline justify-between gap-2 text-xs">
        <span className="truncate text-muted-foreground">{label}</span>
        <span className="shrink-0 font-medium tabular-nums text-foreground">
          {formatPercent(percent)}
        </span>
      </div>
      <Progress
        aria-label={`${label}: ${formatPercent(percent)}`}
        className="h-1 bg-foreground/10"
        value={percent}
      />
      {detail ? (
        <div className="truncate text-xs text-muted-foreground/80">
          {detail}
        </div>
      ) : null}
    </div>
  );
}

function StatusMessage({ status }: { status: AgentStatusViewModel }) {
  const copy =
    status.state === "unconfigured"
      ? "Agent identity not configured"
      : status.state === "loading"
        ? "Loading metrics…"
        : status.state === "error"
          ? status.errorMessage || "Metrics unavailable"
          : "Waiting for metrics";
  const isError = status.state === "error";

  return (
    <div
      className={cn(
        "flex items-center gap-1.5 text-xs",
        isError ? "text-destructive" : "text-muted-foreground",
      )}
      role={isError ? "status" : undefined}
    >
      {isError ? (
        <AlertCircle aria-hidden="true" className="size-3.5 shrink-0" />
      ) : (
        <Clock3 aria-hidden="true" className="size-3.5 shrink-0" />
      )}
      <span className="truncate">{copy}</span>
    </div>
  );
}

function UsageWindow({
  window,
  nowSeconds,
}: {
  window: AgentUsageWindowSnapshot;
  nowSeconds: number;
}) {
  return (
    <MetricBar
      detail={formatReset(window.resetAt, nowSeconds)}
      label={window.label}
      percent={window.usedPercent}
    />
  );
}

function AgentStatusCard({
  compact = false,
  nowSeconds,
  status,
}: {
  compact?: boolean;
  nowSeconds: number;
  status: AgentStatusViewModel;
}) {
  const hasMetrics = status.state === "ready" || status.state === "stale";
  const age = formatDataAge(status.ageSeconds);
  const technicalLabel = [status.model, status.harness]
    .filter(Boolean)
    .join(" · ");

  return (
    <article
      aria-label={`${status.label} usage`}
      className={cn(
        "rounded-xl border border-border/60 bg-background/45",
        compact ? "min-w-44 space-y-1.5 px-2.5 py-2" : "space-y-2.5 p-3",
      )}
    >
      <header className="flex min-w-0 items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <span
            aria-hidden="true"
            className={cn(
              "size-2 shrink-0 rounded-full",
              status.state === "ready"
                ? "bg-emerald-500"
                : status.state === "stale"
                  ? "bg-amber-500"
                  : status.state === "error"
                    ? "bg-destructive"
                    : "bg-muted-foreground/40",
            )}
          />
          <span className="truncate text-sm font-semibold">{status.label}</span>
        </div>
        {status.state === "stale" ? (
          <span className="text-xs font-medium text-amber-600 dark:text-amber-400">
            Stale
          </span>
        ) : null}
      </header>

      {hasMetrics ? (
        <>
          {status.contextPercent !== null ? (
            <MetricBar
              detail={`${formatTokenCount(status.contextUsedTokens)} / ${formatTokenCount(status.contextLimitTokens)} tokens`}
              label="Context"
              percent={status.contextPercent}
            />
          ) : (
            <div className="text-xs text-muted-foreground">
              Context unavailable
            </div>
          )}
          {status.usageWindows.map((window, index) => (
            <UsageWindow
              key={`${window.label}-${index}`}
              nowSeconds={nowSeconds}
              window={window}
            />
          ))}
          <footer className="flex min-w-0 items-center justify-between gap-2 text-xs text-muted-foreground">
            <span className="truncate" title={technicalLabel || undefined}>
              {technicalLabel || "Unknown runtime"}
            </span>
            {age ? <span className="shrink-0">{age}</span> : null}
          </footer>
        </>
      ) : (
        <StatusMessage status={status} />
      )}
    </article>
  );
}

export function AgentStatusSidebarPanel({
  statuses,
  nowSeconds = Math.floor(Date.now() / 1_000),
}: {
  statuses: readonly AgentStatusViewModel[];
  nowSeconds?: number;
}) {
  return (
    <section
      aria-label="Agent status"
      className="hidden space-y-2 border-t border-border/50 px-2 pt-2 md:block"
      data-testid="agent-status-sidebar"
    >
      <div className="flex items-center gap-2 px-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        <Bot aria-hidden="true" className="size-3.5" />
        Agent usage
      </div>
      {statuses.map((status) => (
        <AgentStatusCard
          key={status.id}
          nowSeconds={nowSeconds}
          status={status}
        />
      ))}
    </section>
  );
}

export function AgentStatusMobileBar({
  statuses,
  nowSeconds = Math.floor(Date.now() / 1_000),
}: {
  statuses: readonly AgentStatusViewModel[];
  nowSeconds?: number;
}) {
  return (
    <aside
      aria-label="Agent status"
      className="fixed inset-x-2 bottom-[max(0.5rem,env(safe-area-inset-bottom))] z-30 flex gap-2 overflow-x-auto rounded-2xl border border-border/70 bg-background/90 p-1.5 shadow-lg backdrop-blur-xl md:hidden"
      data-testid="agent-status-mobile"
    >
      {statuses.map((status) => (
        <AgentStatusCard
          compact
          key={status.id}
          nowSeconds={nowSeconds}
          status={status}
        />
      ))}
    </aside>
  );
}

export function PermanentAgentStatusSidebarPanel() {
  const statuses = usePermanentAgentStatuses();
  return <AgentStatusSidebarPanel statuses={statuses} />;
}

export function PermanentAgentStatusMobileBar() {
  const statuses = usePermanentAgentStatuses();
  return <AgentStatusMobileBar statuses={statuses} />;
}
