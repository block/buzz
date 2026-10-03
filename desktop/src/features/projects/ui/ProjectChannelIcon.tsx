import { Folders, Hash, Lock } from "lucide-react";

import { cn } from "@/shared/lib/cn";

/** Projects glyph with a small channel hash nested in the lower right.
 *
 * `private` adds a lock badge. Project homes used to return this icon before
 * the private-channel branch, so a private project channel looked public.
 */
export function ProjectChannelIcon({
  className,
  private: isPrivate = false,
}: {
  className?: string;
  private?: boolean;
}) {
  return (
    <span
      aria-hidden={isPrivate ? undefined : true}
      aria-label={isPrivate ? "Private project channel" : undefined}
      className={cn("relative inline-flex size-4 shrink-0", className)}
      data-testid="project-channel-icon"
      data-visibility={isPrivate ? "private" : "public"}
      role={isPrivate ? "img" : undefined}
    >
      <Folders className="!size-full" />
      <Hash
        className="pointer-events-none absolute -bottom-px -right-px !size-[62.5%]"
        strokeWidth={2.5}
      />
      {isPrivate ? (
        <Lock
          className="pointer-events-none absolute -left-px -top-px !size-[55%]"
          data-testid="project-channel-private-lock"
          strokeWidth={2.75}
        />
      ) : null}
    </span>
  );
}
