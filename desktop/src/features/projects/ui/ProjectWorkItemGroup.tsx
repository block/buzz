import type * as React from "react";

/** Groups related project work-item rows under a subtle status summary. */
export function ProjectWorkItemGroup({
  children,
  count,
  icon,
  label,
}: {
  children: React.ReactNode;
  count: number;
  icon: React.ReactNode;
  label: string;
}) {
  return (
    <section
      className="pt-2 first:pt-0"
      data-project-work-item-status={label}
      data-testid="project-work-item-group"
    >
      <div
        className="mx-2 flex h-9 items-center gap-1.5 rounded-md bg-muted/40 px-2 text-xs text-muted-foreground"
        data-testid="project-work-item-group-header"
      >
        <span className="flex h-4 w-4 items-center justify-center opacity-80">
          {icon}
        </span>
        <span className="font-medium text-foreground/80">{label}</span>
        <span className="tabular-nums text-muted-foreground/65">{count}</span>
      </div>
      <div className="mt-1 space-y-0.5 px-2">{children}</div>
    </section>
  );
}
