import { Loader2, Play, Square } from "lucide-react";

import type { RemoteAgentCardModel, RemoteAgentPreset } from "../types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

type RemoteAgentCardProps = {
  card: RemoteAgentCardModel;
  isPending: boolean;
  defaultPreset: RemoteAgentPreset;
  onArm: () => void;
  onDisarm: () => void;
};

function healthDotClass(health: RemoteAgentCardModel["health"]): string {
  switch (health) {
    case "online":
      return "bg-emerald-500";
    case "stale":
      return "bg-amber-500";
    case "stopped":
      return "bg-rose-500";
    default:
      return "bg-muted-foreground/50";
  }
}

export function RemoteAgentCard({
  card,
  isPending,
  defaultPreset,
  onArm,
  onDisarm,
}: RemoteAgentCardProps) {
  const placeholder = card.seatId.startsWith("(");
  const armBlocked = Boolean(card.bodyLive) || placeholder;

  return (
    <div
      className="flex min-h-[140px] flex-col justify-between rounded-xl border border-border/60 bg-card p-3 shadow-sm"
      data-testid={`remote-agent-card-${card.seatId}`}
    >
      <div className="space-y-1">
        <div className="flex items-center gap-2">
          <span
            aria-hidden
            className={cn(
              "h-2.5 w-2.5 shrink-0 rounded-full",
              healthDotClass(card.health),
            )}
          />
          <p className="truncate text-sm font-semibold text-foreground">
            {card.seatId}
          </p>
        </div>
        <p className="truncate text-2xs text-muted-foreground">
          {card.hostId} · {card.hostRole}
          {card.surfaceKind ? ` · ${card.surfaceKind}` : null}
        </p>
        {card.birthCertShort ? (
          <p
            className="truncate font-mono text-3xs text-muted-foreground/90"
            title={card.birthCertId}
          >
            DNA {card.birthCertShort}
            {card.bodyId ? ` · body ${card.bodyId}` : null}
            {card.leaseEpoch != null && card.leaseEpoch > 0
              ? ` · lease ${card.leaseEpoch}`
              : null}
          </p>
        ) : (
          <p className="truncate text-3xs text-amber-600/90 dark:text-amber-400/90">
            DNA unknown · fill pubkey / PUBLIC.txt
          </p>
        )}
        <p className="truncate text-2xs text-muted-foreground">
          {card.bodyLive
            ? `Online · ${card.hostRole}${card.hostId ? ` · ${card.hostId}` : ""}`
            : card.healthLabel}
          {card.model ? ` · ${card.model}` : ""}
        </p>
        {card.runtimes.length > 0 ? (
          <p className="truncate text-2xs text-muted-foreground/80">
            {card.runtimes.join(" · ")}
          </p>
        ) : null}
        {card.surfaceId ? (
          <p
            className="truncate text-3xs text-muted-foreground/70"
            title={card.surfaceId}
          >
            surface {card.surfaceId}
          </p>
        ) : null}
        {card.projectIds && card.projectIds.length > 0 ? (
          <p className="truncate text-3xs text-muted-foreground/70">
            projects {card.projectIds.join(", ")}
          </p>
        ) : null}
      </div>
      <div className="mt-3 flex items-center gap-2">
        <Button
          className="h-8 flex-1 gap-1 text-xs"
          disabled={isPending || armBlocked}
          size="sm"
          title={
            card.bodyLive
              ? "Body already live on host — refuse dual spawn (409). Stop first or fork new DNA."
              : undefined
          }
          type="button"
          variant="secondary"
          onClick={onArm}
        >
          {isPending ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Play className="h-3.5 w-3.5" />
          )}
          {card.bodyLive ? "Live" : "Arm"}
        </Button>
        <Button
          className="h-8 flex-1 gap-1 text-xs"
          disabled={isPending || placeholder}
          size="sm"
          type="button"
          variant="outline"
          onClick={onDisarm}
        >
          <Square className="h-3.5 w-3.5" />
          Stop
        </Button>
      </div>
      <p className="mt-1 truncate text-3xs text-muted-foreground/70">
        {card.bodyLive
          ? "at-most-one body · dual refused"
          : `preset ${defaultPreset}`}
      </p>
    </div>
  );
}
