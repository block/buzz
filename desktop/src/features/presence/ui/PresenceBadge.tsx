import type * as React from "react";

import type { PresenceStatus } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

import {
  getPresenceDotClassName,
  getPresenceLabel,
} from "@/features/presence/lib/presence";

type PresenceBadgeProps = {
  status: PresenceStatus;
  className?: string;
  label?: string;
} & React.HTMLAttributes<HTMLSpanElement>;

export function PresenceDot({
  status,
  className,
  ...props
}: {
  status: PresenceStatus;
  className?: string;
} & React.HTMLAttributes<HTMLSpanElement>) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "inline-flex h-2.5 w-2.5 shrink-0 rounded-full",
        getPresenceDotClassName(status),
        className,
      )}
      {...props}
    />
  );
}

export function PresenceBadge({
  status,
  className,
  label,
  ...props
}: PresenceBadgeProps) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-2 rounded-full border border-border/80 bg-background/70 px-3 py-1 text-xs font-medium text-muted-foreground",
        className,
      )}
      {...props}
    >
      <PresenceDot status={status} />
      <span>{label ?? getPresenceLabel(status)}</span>
    </span>
  );
}
