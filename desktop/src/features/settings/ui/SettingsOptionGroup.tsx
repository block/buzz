import type * as React from "react";

import { cn } from "@/shared/lib/cn";

export function SettingsOptionGroup({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "overflow-hidden rounded-xl border border-border/70 bg-background/70 shadow-xs divide-y divide-border/55",
        className,
      )}
      {...props}
    />
  );
}

export function SettingsOptionRow({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "flex min-h-16 items-center justify-between gap-4 px-4 py-3 text-sm",
        className,
      )}
      {...props}
    />
  );
}
