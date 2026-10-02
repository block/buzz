import type * as React from "react";

import { THREAD_FOCUS_COLUMN_MAX_WIDTH_PX } from "@/features/channels/lib/threadFocusLayout";

export type ThreadPanelLayoutProps = {
  canResetWidth?: boolean;
  columnMaxWidthPx?: number;
  enterMotion?: boolean;
  headerLeading?: React.ReactNode;
  /** Replaces the default "Thread" label. Channel threads leave this unset. */
  headerTitle?: string;
  headerTitleAriaLabel?: string;
  isFocusMode: boolean;
  isSinglePanelView?: boolean;
  layout?: "standalone" | "split";
  showBackButton?: boolean;
  onHeaderTitleClick?: () => void;
  onResetWidth?: () => void;
  onResizeStart?: React.PointerEventHandler<HTMLButtonElement>;
  splitPaneClamp?: boolean;
  testId?: string;
  transparentChrome?: boolean;
};

type ThreadPanelLayoutOptions = {
  headerLeading?: React.ReactNode;
  isFocusDrawer: boolean;
  isSinglePanelView: boolean;
  useSplitAuxiliaryPane: boolean;
};

/** Maps channel presentation into the shared thread-panel layout contract. */
export function getThreadPanelLayout({
  headerLeading,
  isFocusDrawer,
  isSinglePanelView,
  useSplitAuxiliaryPane,
}: ThreadPanelLayoutOptions): ThreadPanelLayoutProps {
  return isFocusDrawer
    ? {
        columnMaxWidthPx: THREAD_FOCUS_COLUMN_MAX_WIDTH_PX,
        headerLeading,
        isFocusMode: true,
        isSinglePanelView: true,
        layout: "standalone",
        transparentChrome: false,
      }
    : {
        columnMaxWidthPx: undefined,
        headerLeading,
        isFocusMode: false,
        isSinglePanelView: useSplitAuxiliaryPane ? false : isSinglePanelView,
        layout: useSplitAuxiliaryPane ? "split" : "standalone",
        transparentChrome: useSplitAuxiliaryPane,
      };
}
