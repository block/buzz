import type { ReactNode } from "react";

import { cn } from "@/shared/lib/cn";

export function PanelSectionGroup({
  children,
  className,
  description,
  headerAction,
  testId,
  title,
}: {
  children: ReactNode;
  className?: string;
  description?: ReactNode;
  headerAction?: ReactNode;
  testId?: string;
  title?: ReactNode;
}) {
  const hasHeader = Boolean(title || description || headerAction);

  return (
    <section className={cn("space-y-1", className)} data-testid={testId}>
      {hasHeader ? (
        <div
          className="flex min-h-7 items-center gap-3 px-4"
          data-slot="panel-section-header"
        >
          <div className="min-w-0 flex-1">
            {title ? (
              <h2 className="text-xs font-semibold text-muted-foreground/70">
                {title}
              </h2>
            ) : null}
            {description ? (
              <p className="mt-0.5 text-xs font-normal text-muted-foreground/70">
                {description}
              </p>
            ) : null}
          </div>
          {headerAction ? <div className="shrink-0">{headerAction}</div> : null}
        </div>
      ) : null}
      <div
        className="divide-y divide-border/55 overflow-hidden rounded-xl border border-border/70 bg-background/70"
        data-slot="panel-section-card"
      >
        {children}
      </div>
    </section>
  );
}
