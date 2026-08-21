import * as React from "react";
import { Check } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";

export const DEFAULT_PLAYBACK_SPEED = 1;
export const PLAYBACK_SPEEDS = [2, 1.75, 1.5, 1.25, 1, 0.75, 0.5, 0.25];

export function formatPlaybackSpeed(speed: number): string {
  return `${speed}x`;
}

export function isPlaybackSpeedOption(speed: number): boolean {
  return PLAYBACK_SPEEDS.some((option) => option === speed);
}

/** Speed menu shared by the inline player and the review overlay. */
export function PlaybackSpeedControl({
  playbackSpeed,
  onPlaybackSpeedChange,
  size = "inline",
  testId,
}: {
  playbackSpeed: number;
  onPlaybackSpeedChange: (speed: number) => void;
  size?: "inline" | "review";
  testId: string;
}) {
  const [open, setOpen] = React.useState(false);
  const label = formatPlaybackSpeed(playbackSpeed);
  const triggerSizeClass =
    size === "review"
      ? "h-8 min-w-11 rounded-lg px-2 text-xs"
      : "h-7 min-w-10 rounded-md px-1.5 text-2xs";

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          aria-label={`Playback speed: ${label}`}
          className={cn(
            "flex shrink-0 items-center justify-center font-semibold tabular-nums text-white transition-colors hover:bg-white/15 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-white/60",
            triggerSizeClass,
            open && "bg-white/15",
          )}
          data-testid={testId}
          type="button"
          onClick={(event) => event.stopPropagation()}
        >
          {label}
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        className="w-28 border-white/10 bg-black/85 p-1 text-white backdrop-blur-xl"
        data-testid={`${testId}-menu`}
        side="top"
        sideOffset={8}
        onClick={(event) => event.stopPropagation()}
      >
        <div className="px-2 pb-1 pt-1 text-2xs font-medium text-white/55">
          Speed
        </div>
        <div className="grid gap-0.5">
          {PLAYBACK_SPEEDS.map((speed) => {
            const speedLabel = formatPlaybackSpeed(speed);
            const selected = speed === playbackSpeed;
            return (
              <button
                aria-pressed={selected}
                className={cn(
                  "flex h-8 w-full items-center justify-between rounded-lg px-2 text-left text-xs font-medium tabular-nums text-white transition-colors hover:bg-white/15 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-white/60",
                  selected && "bg-white/15",
                )}
                key={speed}
                type="button"
                onClick={(event) => {
                  event.stopPropagation();
                  onPlaybackSpeedChange(speed);
                  setOpen(false);
                }}
              >
                <span>{speedLabel}</span>
                {selected ? <Check className="h-3.5 w-3.5" /> : null}
              </button>
            );
          })}
        </div>
      </PopoverContent>
    </Popover>
  );
}
