import { Check, Loader2 } from "lucide-react";

import type { MeetingPlan } from "@/features/meetings/api";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";

type PlanCardProps = {
  plan: MeetingPlan;
  selected: boolean;
  pending: boolean;
  disabled: boolean;
  onSelect: (plan: string) => void;
};

function titleForPlan(plan: MeetingPlan): string {
  const raw = typeof plan.title === "string" ? plan.title : plan.plan;
  return raw.replace(/[-_]+/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}

function optionalString(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value : null;
}

export function PlanCard({
  plan,
  selected,
  pending,
  disabled,
  onSelect,
}: PlanCardProps) {
  const period = optionalString(plan.period ?? plan.interval);
  const retention = optionalString(plan.recording_retention);

  return (
    <div
      className={cn(
        "flex flex-col gap-3 rounded-xl border p-4",
        selected ? "border-primary bg-primary/5" : "border-border/70",
      )}
      data-testid="meeting-plan-card"
    >
      <div className="flex items-baseline justify-between gap-2">
        <p className="text-sm font-semibold">{titleForPlan(plan)}</p>
        <p className="text-sm tabular-nums">
          {plan.amount_sats.toLocaleString()} sats
          {period ? (
            <span className="text-muted-foreground"> / {period}</span>
          ) : null}
        </p>
      </div>

      <ul className="space-y-1 text-xs text-muted-foreground">
        {typeof plan.room_quota === "number" ? (
          <li>
            {plan.room_quota} room{plan.room_quota === 1 ? "" : "s"}
          </li>
        ) : null}
        <li>
          {plan.can_record
            ? `Recording${retention ? ` · ${retention} retention` : ""}`
            : "No recording"}
        </li>
      </ul>

      <Button
        className="mt-auto"
        disabled={disabled || pending}
        onClick={() => onSelect(plan.plan)}
        size="sm"
        type="button"
        variant={selected ? "default" : "outline"}
      >
        {pending ? (
          <Loader2 className="animate-spin" />
        ) : selected ? (
          <Check />
        ) : null}
        {pending ? "Creating invoice…" : "Choose plan"}
      </Button>
    </div>
  );
}
