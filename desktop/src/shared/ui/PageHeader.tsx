import type { ReactNode } from "react";

import { cn } from "@/shared/lib/cn";

type HeaderProps = {
  /** Optional trailing content (buttons, menus) aligned to the header's end. */
  action?: ReactNode;
  className?: string;
  /** Muted supporting line rendered beneath the title. */
  description?: ReactNode;
  title: ReactNode;
};

/**
 * Level 1 of the shared header ramp: one per page (a Settings card, the Agents
 * page). Renders the page's single `h1`. Pair with {@link SectionHeader} for
 * peer sections within the page and {@link SubsectionLabel} for the smallest
 * grouping labels.
 */
export function PageHeader({
  action,
  className,
  description,
  title,
}: HeaderProps) {
  const copy = (
    <>
      <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
      {description ? (
        <p className="text-base font-normal text-muted-foreground">
          {description}
        </p>
      ) : null}
    </>
  );

  if (action) {
    return (
      <div
        className={cn(
          "flex min-w-0 items-start justify-between gap-4",
          className,
        )}
      >
        <div className="min-w-0 space-y-1">{copy}</div>
        <div className="shrink-0">{action}</div>
      </div>
    );
  }

  return <div className={cn("min-w-0 space-y-1", className)}>{copy}</div>;
}

/**
 * Level 2 of the shared header ramp: a peer section within a page (Agent
 * defaults, Teams, Profile info). Renders an `h2` one step down from
 * {@link PageHeader}.
 */
export function SectionHeader({
  action,
  className,
  description,
  title,
}: HeaderProps) {
  const copy = (
    <>
      <h2 className="text-lg font-semibold tracking-tight">{title}</h2>
      {description ? (
        <p className="text-sm font-normal text-muted-foreground">
          {description}
        </p>
      ) : null}
    </>
  );

  if (action) {
    return (
      <div
        className={cn(
          "flex min-w-0 items-start justify-between gap-3",
          className,
        )}
      >
        <div className="min-w-0 space-y-0.5">{copy}</div>
        <div className="shrink-0">{action}</div>
      </div>
    );
  }

  return <div className={cn("min-w-0 space-y-0.5", className)}>{copy}</div>;
}

/**
 * Level 3 of the shared header ramp: the smallest grouping label (field-group
 * eyebrows, table headers). Replaces the assorted uppercase/tracking variants
 * with one canonical style.
 */
export function SubsectionLabel({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <p
      className={cn(
        "text-2xs font-semibold uppercase tracking-wide text-muted-foreground",
        className,
      )}
    >
      {children}
    </p>
  );
}
