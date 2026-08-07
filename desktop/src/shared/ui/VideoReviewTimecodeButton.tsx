import type * as React from "react";

import { cn } from "@/shared/lib/cn";

const TIMECODE_ACCENT_CLASS =
  "bg-[hsl(var(--buzz-video-review-accent,var(--primary))/0.15)] text-[hsl(var(--buzz-video-review-accent-foreground,var(--buzz-video-review-accent,var(--primary))))]";
const TIMECODE_ACCENT_HOVER_CLASS =
  "hover:bg-[hsl(var(--buzz-video-review-accent,var(--primary))/0.3)]";
const MESSAGE_TIMECODE_ACCENT_CLASS =
  "bg-primary/15 text-primary hover:bg-primary/30";

export function VideoReviewTimecodeButton({
  className,
  onClick,
  surface = "review",
  timecode,
}: {
  className?: string;
  onClick: React.MouseEventHandler<HTMLButtonElement>;
  surface?: "message" | "review";
  timecode: string;
}) {
  return (
    <button
      aria-label={`Jump to ${timecode}`}
      className={cn(
        "inline-flex h-5 shrink-0 items-center rounded px-1.5 align-middle font-mono text-2xs font-semibold outline-hidden transition-colors focus-visible:ring-2 focus-visible:ring-white/60",
        surface === "review"
          ? [TIMECODE_ACCENT_CLASS, TIMECODE_ACCENT_HOVER_CLASS]
          : MESSAGE_TIMECODE_ACCENT_CLASS,
        className,
      )}
      data-testid="video-review-comment-timecode"
      type="button"
      onClick={onClick}
    >
      {timecode}
    </button>
  );
}

export const VIDEO_REVIEW_TIMECODE_ACCENT_CLASS = TIMECODE_ACCENT_CLASS;
