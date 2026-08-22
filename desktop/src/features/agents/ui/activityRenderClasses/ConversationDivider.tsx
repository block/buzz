import type * as React from "react";

import { cn } from "@/shared/lib/cn";

/**
 * Quiet centered rule used by the `conversation` variant for items that mark
 * structure rather than work: session boundaries and turn setup/status.
 *
 * VISION_ACTIVITY's "failures rise; reads recede" applies to salience, not just
 * color — in focus mode these items recede to a hairline with a small centered
 * caption so the reader's eye stays on the prompt/response rhythm. Errors and
 * permission gates keep their loud treatment and never route here.
 */
export function ConversationDivider({
  className,
  icon,
  label,
  testId,
  title,
}: {
  className?: string;
  icon?: React.ReactNode;
  label: string;
  testId?: string;
  title?: string;
}) {
  return (
    <div
      className={cn("flex items-center gap-2 py-1", className)}
      data-testid={testId}
      data-variant="conversation-divider"
      title={title}
    >
      <div className="h-px flex-1 bg-border/60" />
      <span className="flex min-w-0 items-center gap-1 text-xs text-muted-foreground/70">
        {icon}
        <span className="truncate">{label}</span>
      </span>
      <div className="h-px flex-1 bg-border/60" />
    </div>
  );
}
