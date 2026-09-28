import * as React from "react";

import { cn } from "@/shared/lib/cn";
import {
  bestieDueCountdownTickMs,
  formatBestieDueCountdown,
} from "./bestieDueCountdown";
import { useBestieDueChipHorizonMinutes } from "./bestieDueChipHorizonPreference";

/**
 * Rough aggregate due chip (`7m` / `45s` / `due`). Seconds ticker only when
 * under a minute; minutes refresh on a coarse cadence. Hidden beyond horizon.
 */
export function BestieDueCountdownChip({
  className,
  dueAt,
  testId,
}: {
  className?: string;
  /** Unix seconds, or null when no due. */
  dueAt: number | null;
  testId?: string;
}) {
  const horizonMinutes = useBestieDueChipHorizonMinutes();
  const horizonSeconds = horizonMinutes * 60;
  const [nowSeconds, setNowSeconds] = React.useState(() =>
    Math.floor(Date.now() / 1000),
  );

  React.useEffect(() => {
    if (dueAt == null || horizonMinutes <= 0) return;
    let timeoutId: number | null = null;
    let cancelled = false;

    const schedule = () => {
      if (cancelled) return;
      const now = Math.floor(Date.now() / 1000);
      setNowSeconds(now);
      const delay = bestieDueCountdownTickMs(dueAt, now, horizonSeconds);
      if (delay == null) return;
      timeoutId = window.setTimeout(schedule, delay);
    };

    schedule();
    return () => {
      cancelled = true;
      if (timeoutId != null) window.clearTimeout(timeoutId);
    };
  }, [dueAt, horizonMinutes, horizonSeconds]);

  if (dueAt == null || horizonMinutes <= 0) return null;
  const label = formatBestieDueCountdown(dueAt, nowSeconds, horizonSeconds);
  if (!label) return null;

  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center rounded-full bg-amber-500/15 px-1.5 py-0.5 text-2xs font-medium tabular-nums text-amber-700 dark:text-amber-400",
        label === "due" && "bg-amber-500/25",
        className,
      )}
      data-testid={testId ?? "bestie-due-chip"}
    >
      {label}
    </span>
  );
}
