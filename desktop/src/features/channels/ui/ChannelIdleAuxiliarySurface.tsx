import type * as React from "react";

import { FocusThreadDrawer } from "@/features/channels/ui/FocusThreadDrawer";
import {
  IdleAuxiliaryPanel,
  type IdleAuxiliaryHeaderControls,
} from "@/features/channels/ui/IdleAuxiliaryPanel";

type ChannelIdleAuxiliarySurfaceProps = {
  canResetWidth: boolean;
  channelName: string;
  children: React.ReactNode;
  headerControls?: IdleAuxiliaryHeaderControls;
  /** Render the panel inside the cover drawer instead of the split pane. */
  isCoverDrawer: boolean;
  isSinglePanelView: boolean;
  onClose: () => void;
  onResetWidth: () => void;
  onResizeStart: React.PointerEventHandler<HTMLButtonElement>;
  title: string;
  useSplitAuxiliaryPane: boolean;
  widthPx: number;
  /**
   * Applies the split-pane presentation, including its resize affordances.
   * Supplied by `ChannelPane` because that pane owns the resize state; the
   * cover-drawer presentation is applied here.
   */
  wrapSplitPane: (panel: React.ReactNode) => React.ReactNode;
};

/**
 * The channel's caller-owned idle auxiliary surface, in whichever presentation
 * was resolved for it.
 *
 * Split out of `ChannelPane` alongside `ChannelAgentSessionSurface` so each
 * auxiliary surface owns its own presentation wiring rather than adding another
 * pair of closures to that component.
 *
 * It reuses `FocusThreadDrawer` rather than `CoverDrawer` directly, so the two
 * cover presentations stay one surface with one test id; only the accessible
 * label differs, which is why that label is a prop on the thread drawer.
 */
export function ChannelIdleAuxiliarySurface({
  canResetWidth,
  channelName,
  children,
  headerControls,
  isCoverDrawer,
  isSinglePanelView,
  onClose,
  onResetWidth,
  onResizeStart,
  title,
  useSplitAuxiliaryPane,
  widthPx,
  wrapSplitPane,
}: ChannelIdleAuxiliarySurfaceProps) {
  const panel = (
    <IdleAuxiliaryPanel
      canResetWidth={canResetWidth}
      headerControls={headerControls}
      isFocusDrawer={isCoverDrawer}
      isSinglePanelView={isSinglePanelView}
      onClose={onClose}
      onResetWidth={onResetWidth}
      onResizeStart={onResizeStart}
      title={title}
      useSplitAuxiliaryPane={useSplitAuxiliaryPane}
      widthPx={widthPx}
    >
      {children}
    </IdleAuxiliaryPanel>
  );

  return isCoverDrawer ? (
    <FocusThreadDrawer
      channelName={channelName}
      label={title || "Panel"}
      onClose={onClose}
    >
      {panel}
    </FocusThreadDrawer>
  ) : (
    wrapSplitPane(panel)
  );
}
