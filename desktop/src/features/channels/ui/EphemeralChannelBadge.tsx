import { ClockFading } from "lucide-react";

import type { EphemeralChannelDisplay } from "@/features/channels/lib/ephemeralChannel";
import { cn } from "@/shared/lib/cn";

type EphemeralChannelBadgeProps = {
  display: EphemeralChannelDisplay;
  testId?: string;
  variant: "header" | "sidebar";
};

export function EphemeralChannelBadge({
  display,
  testId,
  variant,
}: EphemeralChannelBadgeProps) {
  const isHeader = variant === "header";

  return (
    <ClockFading
      aria-label={display.tooltipLabel}
      className={cn(
        "h-4 w-4 shrink-0",
        isHeader ? "text-muted-foreground" : "text-sidebar-foreground/45",
      )}
      data-testid={testId}
      role="img"
    />
  );
}
