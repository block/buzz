import type * as React from "react";

import { cn } from "@/shared/lib/cn";

/**
 * Horizontal drag strip along a composer's top edge. Doubles as the pane
 * border; dragging (or ↑/↓ when focused) resizes the composer.
 */
export function DevComposerResizeHandle({
  dragging,
  testId,
  "aria-valuenow": valueNow,
  ...handlers
}: {
  dragging: boolean;
  testId: string;
  "aria-valuenow": number;
  onPointerDown: (event: React.PointerEvent<HTMLDivElement>) => void;
  onPointerMove: (event: React.PointerEvent<HTMLDivElement>) => void;
  onPointerUp: (event: React.PointerEvent<HTMLDivElement>) => void;
  onKeyDown: (event: React.KeyboardEvent<HTMLDivElement>) => void;
}) {
  return (
    // biome-ignore lint/a11y/useSemanticElements: <hr> cannot host the drag/keyboard resize handlers of a movable separator
    <div
      className={cn(
        "h-1 w-full shrink-0 cursor-row-resize bg-border/60 outline-none hover:bg-primary/60 focus-visible:bg-primary/60",
        dragging && "bg-primary",
      )}
      data-testid={testId}
      role="separator"
      aria-orientation="horizontal"
      aria-valuenow={valueNow}
      tabIndex={0}
      {...handlers}
    />
  );
}
