import type * as React from "react";

import { THREAD_FOCUS_COLUMN_MAX_WIDTH_PX } from "@/features/channels/lib/threadFocusLayout";
import type { ChannelType } from "@/shared/api/types";
import { AUXILIARY_PANEL_SINGLE_COLUMN_BREAKPOINT_PX } from "@/shared/layout/AuxiliaryPanel";

const HEADER_ACTIONS_COMPACT_BREAKPOINT_PX = 760;

export type ThreadPanelLayoutProps = {
  columnMaxWidthPx?: number;
  headerLeading?: React.ReactNode;
  isFocusMode: boolean;
  isSinglePanelView?: boolean;
  layout?: "standalone" | "split";
  transparentChrome?: boolean;
};

type ThreadPanelLayoutOptions = {
  headerLeading?: React.ReactNode;
  isFocusDrawer: boolean;
  isSinglePanelView: boolean;
  useSplitAuxiliaryPane: boolean;
};

type ChannelScreenPanelLayoutOptions = {
  appActive: boolean;
  auxiliaryPanelRequested: boolean;
  channelType?: ChannelType;
  contentWidthPx: number;
};

export function getScreenLayout({
  appActive,
  auxiliaryPanelRequested,
  channelType,
  contentWidthPx,
}: ChannelScreenPanelLayoutOptions) {
  const hasAuxiliaryPanel = !appActive && auxiliaryPanelRequested;
  const isNarrowPanelViewport =
    contentWidthPx > 0 &&
    contentWidthPx < AUXILIARY_PANEL_SINGLE_COLUMN_BREAKPOINT_PX;
  return {
    isSinglePanelView:
      isNarrowPanelViewport && channelType !== "forum" && hasAuxiliaryPanel,
    shouldCompactHeaderActions:
      hasAuxiliaryPanel &&
      contentWidthPx > 0 &&
      contentWidthPx < HEADER_ACTIONS_COMPACT_BREAKPOINT_PX,
  };
}

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
