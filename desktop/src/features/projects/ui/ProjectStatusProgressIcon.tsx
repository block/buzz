import type * as React from "react";

import { cn } from "@/shared/lib/cn";

export type ProjectStatusProgressState =
  | "queued"
  | "started"
  | "review"
  | "completed"
  | "canceled";

/** Linear-style circular indicator showing a work item's workflow progress. */
export function ProjectStatusProgressIcon({
  "aria-label": ariaLabel,
  className,
  state,
  ...props
}: React.SVGProps<SVGSVGElement> & {
  state: ProjectStatusProgressState;
}) {
  return (
    <svg
      aria-label={ariaLabel ?? `${state} status`}
      className={cn("shrink-0", className)}
      data-project-status-progress={state}
      fill="none"
      role="img"
      viewBox="0 0 16 16"
      {...props}
    >
      {state === "queued" ? (
        <circle
          cx="8"
          cy="8"
          r="6"
          stroke="currentColor"
          strokeDasharray="1.5 1.8"
          strokeLinecap="round"
          strokeWidth="1.5"
        />
      ) : null}
      {state === "started" || state === "review" ? (
        <>
          <circle
            cx="8"
            cy="8"
            opacity="0.45"
            r="6"
            stroke="currentColor"
            strokeWidth="1.5"
          />
          <path
            d={
              state === "started"
                ? "M8 8V2A6 6 0 0 1 8 14Z"
                : "M8 8V2A6 6 0 1 1 2 8Z"
            }
            fill="currentColor"
            opacity="0.8"
          />
        </>
      ) : null}
      {state === "completed" || state === "canceled" ? (
        <>
          <circle className="fill-current" cx="8" cy="8" r="6.5" />
          <path
            className="stroke-background"
            d={
              state === "completed"
                ? "m4.75 8.1 2 2 4.5-4.5"
                : "m5.5 5.5 5 5m0-5-5 5"
            }
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth="1.5"
          />
        </>
      ) : null}
    </svg>
  );
}
