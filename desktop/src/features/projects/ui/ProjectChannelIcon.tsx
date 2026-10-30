import { Folders, Hash } from "lucide-react";

import { cn } from "@/shared/lib/cn";

/** Projects glyph with a small channel hash nested in the lower right. */
export function ProjectChannelIcon({ className }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn("relative inline-flex size-4 shrink-0", className)}
      data-testid="project-channel-icon"
    >
      <Folders className="!size-full" />
      <Hash
        className="pointer-events-none absolute -bottom-px -right-px !size-[62.5%]"
        strokeWidth={2.5}
      />
    </span>
  );
}
