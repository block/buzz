import { CircleAlert, CircleDashed } from "lucide-react";
import type * as React from "react";

import { cn } from "@/shared/lib/cn";

export function ProjectPanelState({
  action,
  className,
  description,
  error = false,
  panel = true,
  testId,
  title,
}: {
  action?: React.ReactNode;
  className?: string;
  description?: string;
  error?: boolean;
  panel?: boolean;
  testId?: string;
  title: string;
}) {
  const Icon = error ? CircleAlert : CircleDashed;
  return (
    <div
      className={cn(
        "flex min-h-64 w-full flex-1 flex-col items-center justify-center gap-3 px-6 py-12 text-center",
        className,
      )}
      data-project-detail-panel={panel ? "" : undefined}
      data-testid={testId}
    >
      <Icon className="h-9 w-9 text-muted-foreground/35" />
      <div className="max-w-sm space-y-1">
        <p className="text-sm font-medium text-foreground">{title}</p>
        {description ? (
          <p className="text-sm leading-5 text-muted-foreground">
            {description}
          </p>
        ) : null}
      </div>
      {action}
    </div>
  );
}
