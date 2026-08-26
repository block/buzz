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
  const hasHeader = Boolean(title || description || headerAction);

  return (
    <div className={cn("space-y-2", className)} {...props}>
      {hasHeader ? (
        <div
          className="flex min-h-7 items-end gap-3 px-4"
          data-slot="settings-section-header"
        >
          <div className="min-w-0 flex-1">
            {title ? (
              <h2 className="text-sm font-semibold text-muted-foreground/70">
                {title}
              </h2>
            ) : null}
            {description ? (
              <p
                className="mt-0.5 text-sm font-normal text-muted-foreground/70"
                data-settings-subcopy
              >
                {description}
              </p>
            ) : null}
          </div>
          {headerAction ? <div className="shrink-0">{headerAction}</div> : null}
        </div>
      ) : null}
      <div
        className={cn(
          "[container-type:inline-size]",
          surface === "soft"
            ? "overflow-hidden rounded-2xl bg-muted/20"
            : "divide-y divide-border/55 overflow-hidden rounded-xl border border-border/70 bg-background/70",
        )}
        data-slot="settings-section-card"
      >
        {children}
      </div>
    </div>
  );
}

export function SettingsOptionGroupList({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn("space-y-12", className)}
      data-slot="settings-section-list"
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
        "flex min-h-16 items-center justify-between gap-4 px-4 py-3 text-sm [@container(max-width:34rem)]:flex-col [@container(max-width:34rem)]:items-start [@container(max-width:34rem)]:[&>[data-slot=segmented-control]]:w-full",
        className,
      )}
      {...props}
    />
  );
}
