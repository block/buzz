import * as React from "react";

import { cn } from "@/shared/lib/cn";

/**
 * shadcn-style progress bar (no Radix dependency). `value` is 0–100; a
 * `null`/`undefined` value renders an indeterminate sweep, used for phases
 * with no byte counts (e.g. video transcoding before the upload starts).
 */
const Progress = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement> & { value?: number | null }
>(({ className, value, ...props }, ref) => {
  const clamped =
    typeof value === "number" && Number.isFinite(value)
      ? Math.min(100, Math.max(0, value))
      : null;

  return (
    <div
      aria-valuemax={100}
      aria-valuemin={0}
      aria-valuenow={clamped ?? undefined}
      className={cn(
        "relative h-2 w-full overflow-hidden rounded-full bg-primary/20",
        className,
      )}
      ref={ref}
      role="progressbar"
      {...props}
    >
      {clamped === null ? (
        <div className="h-full w-1/3 animate-[progress-indeterminate_1.2s_ease-in-out_infinite] rounded-full bg-primary" />
      ) : (
        <div
          className="h-full w-full flex-1 rounded-full bg-primary transition-transform duration-200"
          style={{ transform: `translateX(-${100 - clamped}%)` }}
        />
      )}
    </div>
  );
});
Progress.displayName = "Progress";

export { Progress };
