import type * as React from "react";

import { ProjectContextRail } from "@/features/projects/ui/ProjectContextRail";
import { ProjectHomeColumn } from "@/features/projects/ui/ProjectHomeColumn";
import { PROJECT_CONTEXT_PANEL_DEFAULT_WIDTH_PX } from "@/features/projects/ui/useProjectPanelWidths";
import { useThreadPanelWidth } from "@/shared/hooks/useThreadPanelWidth";
import { SIDEBAR_WIDTH_MIN } from "@/shared/layout/sidebarLayout";
import { cn } from "@/shared/lib/cn";

const BESTIE_DM_CONTEXT_WIDTH_KEY = "buzz.desktop.bestie-dm-context-width";

/**
 * Project-home layout mirror for Bestie DM: conversation on the left, fixed
 * category column on the right. Category item sheets use ChannelScreen
 * idleAuxiliary (slide), not this column.
 */
export function BestieDmChannelFrame({
  children,
  column,
  open,
}: {
  children: React.ReactNode;
  column: React.ReactNode;
  open: boolean;
}) {
  const summaryWidth = useThreadPanelWidth(undefined, {
    defaultWidthPx: PROJECT_CONTEXT_PANEL_DEFAULT_WIDTH_PX,
    minWidthPx: SIDEBAR_WIDTH_MIN,
    sessionKey: BESTIE_DM_CONTEXT_WIDTH_KEY,
  });

  return (
    <div
      className={cn(
        "relative flex min-h-0 min-w-0 flex-1 overflow-hidden",
        open && "bg-sidebar",
      )}
      data-testid="bestie-dm-channel-frame"
    >
      <div
        className={cn(
          "relative flex min-h-0 min-w-60 flex-1 flex-col overflow-hidden",
          open ? "mb-2 ml-px mt-px rounded-2xl bg-background" : "bg-muted/20",
        )}
      >
        {children}
      </div>
      <ProjectContextRail
        open={open}
        panelWidthPx={summaryWidth.widthPx}
        resizing={summaryWidth.isResizing}
        rounded={false}
        testId="bestie-dm-context-rail"
      >
        {open ? (
          <ProjectHomeColumn
            bodyClassName="overflow-y-auto overflow-x-hidden overscroll-contain"
            canResetWidth={summaryWidth.canReset}
            onResetWidth={summaryWidth.onResetWidth}
            onResizeStart={summaryWidth.onResizeStart}
            testId="bestie-dm-context-column"
            widthPx={summaryWidth.widthPx}
          >
            {column}
          </ProjectHomeColumn>
        ) : null}
      </ProjectContextRail>
    </div>
  );
}
