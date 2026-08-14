import { ArrowUpRight } from "lucide-react";

import { cn } from "@/shared/lib/cn";

/**
 * Hover affordance for transcript message bubbles that navigate to the
 * original message in chat. Rendered inside a `group/bubble` container; fades
 * in on hover or keyboard focus of the bubble.
 */
export function MessageLinkHoverCue({ className }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "pointer-events-none absolute right-1.5 top-1.5 inline-flex items-center gap-0.5 rounded-md bg-background/95 py-0.5 pl-1.5 pr-1 text-2xs font-medium text-muted-foreground",
        "opacity-0 transition-opacity duration-150 group-hover/bubble:opacity-100 group-focus-visible/bubble:opacity-100",
        className,
      )}
    >
      Open in chat
      <ArrowUpRight className="h-3 w-3" />
    </span>
  );
}
