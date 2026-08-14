import type * as React from "react";

import { cn } from "@/shared/lib/cn";

export function SettingsOptionGroup({
  children,
  className,
  description,
  headerAction,
  surface = "framed",
  title,
  ...props
}: Omit<React.HTMLAttributes<HTMLDivElement>, "title"> & {
  description?: React.ReactNode;
  headerAction?: React.ReactNode;
  surface?: "framed" | "soft";
  title?: React.ReactNode;
}) {
  return (
    <div
      className={cn(
        surface === "soft"
          ? "overflow-hidden rounded-2xl bg-muted/20"
          : "overflow-hidden rounded-xl border border-border/70 bg-background/70 divide-y divide-border/55",
        className,
      )}
      {...props}
    >
      {title ? (
        <div className="flex min-h-14 items-center justify-between gap-4 px-4 py-3">
          <div className="min-w-0">
            <h2 className="text-lg font-semibold tracking-tight">{title}</h2>
            {description ? (
              <p
                className="mt-1 text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                {description}
              </p>
            ) : null}
          </div>
          {headerAction ? <div className="shrink-0">{headerAction}</div> : null}
        </div>
      ) : null}
      {children}
    </div>
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
