import type { LucideIcon } from "lucide-react";
import type * as React from "react";

import { cn } from "@/shared/lib/cn";

const META_ROW_CLASS =
  "flex min-h-7 min-w-0 items-center gap-2 py-1 text-xs leading-4";

/** Chip height matches the meta row's text/avatar line so Labels isn't taller. */
export const PROJECT_DETAIL_META_PILL_CLASS =
  "inline-flex h-5 items-center rounded-full border border-border/60 px-1.5 text-2xs leading-none text-muted-foreground";

/** Labeled icon rows under the work-item title, in a padded muted card. */
export function ProjectDetailMetaList({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "mx-6 mb-4 mt-2 rounded-lg bg-muted/35 px-3 py-1.5",
        className,
      )}
      data-testid="project-detail-meta"
    >
      {children}
    </div>
  );
}

export function ProjectDetailMetaRow({
  children,
  icon: Icon,
  label,
  labelClassName,
  leading,
}: {
  children: React.ReactNode;
  icon?: LucideIcon;
  label: string;
  labelClassName?: string;
  leading?: React.ReactNode;
}) {
  return (
    <div className={META_ROW_CLASS}>
      {leading ??
        (Icon ? (
          <Icon className="h-4 w-4 shrink-0 text-muted-foreground" />
        ) : (
          <span className="h-4 w-4 shrink-0" />
        ))}
      <span
        className={cn(
          "w-20 shrink-0 truncate whitespace-nowrap font-normal text-muted-foreground/75",
          labelClassName,
        )}
        title={label}
      >
        {label}
      </span>
      <div className="flex min-h-5 min-w-0 flex-1 items-center font-medium text-foreground">
        {children}
      </div>
    </div>
  );
}

export function ProjectDetailMetaPills({ labels }: { labels: string[] }) {
  return (
    <div className="flex flex-wrap items-center gap-1">
      {labels.map((label) => (
        <span className={PROJECT_DETAIL_META_PILL_CLASS} key={label}>
          {label}
        </span>
      ))}
    </div>
  );
}
