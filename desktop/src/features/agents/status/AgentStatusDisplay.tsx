import { AlertCircle, Bot, Clock3 } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
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

function primaryPercent(status: AgentStatusViewModel): number | null {
  const knownPercents = status.usageWindows.map((window) => window.usedPercent);
  if (status.contextPercent !== null) {
    knownPercents.push(status.contextPercent);
  }
  return knownPercents.length > 0 ? Math.max(...knownPercents) : null;
}

function MetricBar({
  label,
  percent,
  detail,
}: {
  label: string;
  percent: number;
  detail?: string | null;
}) {
  return (
    <div className="space-y-1">
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

export function AgentStatusIndicator({
  className,
  status,
}: {
  className?: string;
  status: AgentStatusViewModel;
}) {
  const percent = primaryPercent(status);
  const label =
    percent === null
      ? `${status.label} usage unavailable`
      : `${status.label} usage: ${formatPercent(percent)}`;

  return (
    <span
      aria-label={label}
      className={cn(
        "relative inline-flex size-8 shrink-0 items-center justify-center rounded-full text-[9px] font-semibold leading-none tabular-nums",
        status.state === "stale"
          ? "text-amber-600 dark:text-amber-400"
          : status.state === "error"
            ? "text-destructive"
            : "text-muted-foreground",
        className,
      )}
      role="img"
      title={label}
    >
      <svg aria-hidden="true" className="absolute inset-0 size-full -rotate-90">
        <circle
          className="stroke-current opacity-15"
          cx="16"
          cy="16"
          fill="none"
          r="13"
          strokeWidth="2"
        />
        {percent !== null ? (
          <circle
            className="stroke-current"
            cx="16"
            cy="16"
            fill="none"
            pathLength="100"
            r="13"
            strokeDasharray="100"
            strokeDashoffset={100 - percent}
            strokeLinecap="round"
            strokeWidth="2"
          />
        ) : null}
      </svg>
      <span>{percent === null ? "—" : formatPercent(percent)}</span>
    </span>
  );
}

export function AgentStatusDetails({
  nowSeconds = Math.floor(Date.now() / 1_000),
  status,
}: {
  nowSeconds?: number;
  status: AgentStatusViewModel;
}) {
  const hasMetrics = status.state === "ready" || status.state === "stale";
  const age = formatDataAge(status.ageSeconds);
  const technicalLabel = [status.model, status.harness]
    .filter(Boolean)
    .join(" · ");
  const usageWindowOccurrences = new Map<string, number>();

  return (
    <div className="w-64 space-y-3" data-testid="agent-status-details">
      <header className="flex min-w-0 items-center justify-between gap-2">
        <span className="truncate text-sm font-semibold">{status.label}</span>
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
          {status.usageWindows.map((window) => {
            const occurrence = usageWindowOccurrences.get(window.label) ?? 0;
            usageWindowOccurrences.set(window.label, occurrence + 1);
            return (
              <UsageWindow
                key={`${window.label}-${occurrence}`}
                nowSeconds={nowSeconds}
                window={window}
              />
            );
          })}
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
    </div>
  );
}

export function AgentStatusPopover({
  className,
  nowSeconds,
  status,
}: {
  className?: string;
  nowSeconds?: number;
  status: AgentStatusViewModel;
}) {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          className={cn(
            "rounded-full outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
            className,
          )}
          type="button"
        >
          <AgentStatusIndicator status={status} />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-auto p-3">
        <AgentStatusDetails nowSeconds={nowSeconds} status={status} />
      </PopoverContent>
    </Popover>
  );
}

export function AgentStatusSidebarPanel({
  statuses,
  nowSeconds = Math.floor(Date.now() / 1_000),
}: {
  statuses: readonly AgentStatusViewModel[];
  nowSeconds?: number;
}) {
  if (statuses.length === 0) return null;
  return (
    <section
      aria-label="Agent usage"
      className="space-y-1 border-t border-border/50 px-2 pt-2 group-data-[collapsible=icon]:hidden"
      data-testid="agent-status-sidebar"
    >
      <div className="flex items-center gap-2 px-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
        <Bot aria-hidden="true" className="size-3.5" />
        Agent usage
      </div>
      {statuses.map((status) => (
        <div
          className="flex min-w-0 items-center justify-between gap-2 rounded-lg px-1.5 py-0.5"
          key={status.id}
        >
          <span className="truncate text-xs font-medium">{status.label}</span>
          <AgentStatusPopover nowSeconds={nowSeconds} status={status} />
        </div>
      ))}
    </section>
  );
}

export function PermanentAgentStatusSidebarPanel() {
  const statuses = usePermanentAgentStatuses();
  return <AgentStatusSidebarPanel statuses={statuses} />;
}

export function AgentStatusForPubkey({ pubkey }: { pubkey?: string }) {
  const statuses = usePermanentAgentStatuses();
  const normalizedPubkey = pubkey?.trim().toLowerCase();
  const status = normalizedPubkey
    ? statuses.find(
        (candidate) =>
          candidate.agentPubkey?.toLowerCase() === normalizedPubkey,
      )
    : undefined;
  return status ? <AgentStatusPopover status={status} /> : null;
}
