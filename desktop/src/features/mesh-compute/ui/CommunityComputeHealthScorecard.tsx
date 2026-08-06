import {
  AlertTriangle,
  CheckCircle2,
  CircleHelp,
  MinusCircle,
  XCircle,
} from "lucide-react";

import { cn } from "@/shared/lib/cn";
import type {
  CommunityComputeDeploymentHealth,
  CommunityComputeHealthSummary,
} from "../communityComputeMapModel";

export type CommunityComputeHealthScorecardProps = {
  health: CommunityComputeHealthSummary;
  className?: string;
};

const HEALTH_ROWS: readonly {
  status: CommunityComputeDeploymentHealth;
  label: string;
}[] = [
  { status: "healthy", label: "Healthy" },
  { status: "warming", label: "Warming" },
  { status: "idle", label: "Idle" },
  { status: "degraded", label: "Degraded" },
  { status: "offline", label: "Offline" },
  { status: "unknown", label: "Unknown" },
];

/** Health summary with icon, copy, and count labels that do not rely on color. */
export function CommunityComputeHealthScorecard({
  health,
  className,
}: CommunityComputeHealthScorecardProps) {
  const Icon = statusIcon(health.status);
  const total = Object.values(health.counts).reduce(
    (sum, count) => sum + count,
    0,
  );

  return (
    <section
      aria-labelledby="community-compute-health-title"
      className={cn(
        "rounded-xl border border-border/70 bg-background p-4",
        className,
      )}
      data-status={health.status}
      data-testid="community-compute-health-scorecard"
    >
      <div className="flex items-start gap-3">
        <span
          aria-hidden="true"
          className={cn(
            "flex size-9 shrink-0 items-center justify-center rounded-full border bg-muted/30",
            health.status === "healthy" && "border-foreground/30",
            health.status === "attention" &&
              "border-dashed border-foreground/50",
            health.status === "critical" && "border-2 border-foreground",
          )}
        >
          <Icon className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          <p
            className="text-sm font-semibold"
            data-testid="community-compute-health-label"
            id="community-compute-health-title"
          >
            {health.label}
          </p>
          <p className="mt-0.5 text-xs leading-snug text-muted-foreground">
            {health.reason ?? healthDescription(health.status, total)}
          </p>
        </div>
        <span className="shrink-0 text-xs font-medium tabular-nums text-muted-foreground">
          {total} total
        </span>
      </div>

      <dl className="mt-4 grid grid-cols-3 gap-2 sm:grid-cols-6">
        {HEALTH_ROWS.map(({ status, label }) => (
          <div
            className={cn(
              "rounded-lg border border-border/60 px-2 py-2 text-center",
              health.counts[status] > 0 && healthClass(status),
            )}
            data-testid={`community-compute-health-${status}`}
            key={status}
          >
            <dt className="truncate text-2xs text-muted-foreground">{label}</dt>
            <dd className="mt-0.5 text-sm font-semibold tabular-nums">
              {health.counts[status]}
            </dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

function statusIcon(status: CommunityComputeHealthSummary["status"]) {
  switch (status) {
    case "healthy":
      return CheckCircle2;
    case "attention":
      return AlertTriangle;
    case "critical":
      return XCircle;
    case "empty":
      return MinusCircle;
    case "unknown":
      return CircleHelp;
  }
}

function healthClass(status: CommunityComputeDeploymentHealth): string {
  switch (status) {
    case "healthy":
      return "border-solid bg-foreground/[0.04]";
    case "warming":
      return "border-dashed bg-foreground/[0.03]";
    case "idle":
      return "border-dotted";
    case "degraded":
      return "border-dashed border-foreground/50";
    case "offline":
      return "border-foreground/50 bg-muted/50 line-through decoration-muted-foreground/50";
    case "unknown":
      return "border-dotted bg-muted/20";
  }
}

function healthDescription(
  status: CommunityComputeHealthSummary["status"],
  total: number,
): string {
  switch (status) {
    case "healthy":
      return "No reporting device needs attention; some may still be warming or idle.";
    case "attention":
      return "Some deployments may have reduced availability.";
    case "critical":
      return "Shared compute is currently unavailable.";
    case "empty":
      return "No deployments have joined this community yet.";
    case "unknown":
      return total === 0
        ? "Waiting for deployment health reports."
        : "Deployment health could not be determined.";
  }
}
