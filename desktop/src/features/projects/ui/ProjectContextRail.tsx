import type * as React from "react";

import { cn } from "@/shared/lib/cn";

const CONTEXT_RAIL_GUTTER_PX = 8;

export function ProjectContextRail({
  animateWidth = true,
  children,
  open,
  panelWidthPx,
  resizing = false,
  rounded = true,
  testId = "project-context-rail",
}: {
  animateWidth?: boolean;
  children: React.ReactNode;
  open: boolean;
  panelWidthPx: number;
  resizing?: boolean;
  rounded?: boolean;
  testId?: string;
}) {
  return (
    <div
      aria-hidden={!open}
      className={cn(
        "relative z-30 h-full shrink-0 overflow-hidden motion-reduce:transition-none",
        resizing || !animateWidth
          ? "transition-none"
          : "transition-[width] duration-200 ease-linear",
      )}
      data-resizing={resizing ? "true" : "false"}
      data-testid={testId}
      style={{
        contain: "layout paint style",
        width: open ? panelWidthPx + CONTEXT_RAIL_GUTTER_PX : 0,
      }}
    >
      <div
        className={cn(
          "absolute inset-y-0 left-2 overflow-hidden",
          rounded && "rounded-2xl",
        )}
        data-testid={`${testId}-panel`}
        style={{ width: panelWidthPx }}
      >
        {children}
      </div>
    </div>
  );
}
