import type * as React from "react";

import { THREAD_FOCUS_COLUMN_MAX_WIDTH_PX } from "@/features/channels/lib/threadFocusLayout";

type ThreadPanelLayoutOptions = {
  headerLeading?: React.ReactNode;
  isFocusDrawer: boolean;
  isSinglePanelView: boolean;
  useSplitAuxiliaryPane: boolean;
};

/**
 * Maps the channel-level thread presentation into the shared auxiliary panel's
 * layout contract. Focus mode fills its drawer and owns its chrome; narrow
 * viewports keep the existing standalone behavior because no split destination
 * is available there.
 */
export function getThreadPanelLayout({
  headerLeading,
  isFocusDrawer,
  isSinglePanelView,
  useSplitAuxiliaryPane,
}: ThreadPanelLayoutOptions) {
  return isFocusDrawer
    ? ({
        columnMaxWidthPx: THREAD_FOCUS_COLUMN_MAX_WIDTH_PX,
        headerLeading,
        isSinglePanelView: true,
        layout: "standalone",
        transparentChrome: false,
      } as const)
    : ({
        columnMaxWidthPx: undefined,
        headerLeading,
        isSinglePanelView: useSplitAuxiliaryPane ? false : isSinglePanelView,
        layout: useSplitAuxiliaryPane ? "split" : "standalone",
        transparentChrome: useSplitAuxiliaryPane,
      } as const);
}
