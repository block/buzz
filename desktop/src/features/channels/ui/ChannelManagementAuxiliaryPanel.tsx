import type * as React from "react";

import type { Channel } from "@/shared/api/types";
import { ChannelManagementSheet } from "@/features/channels/ui/ChannelManagementSheet";
import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";

type ChannelManagementAuxiliaryPanelProps = {
  activeChannel: Channel;
  canResetThreadPanelWidth: boolean;
  currentPubkey?: string;
  isSinglePanelView: boolean;
  onChannelManagementDeleted?: () => void;
  onCloseChannelManagement?: () => void;
  onResetThreadPanelWidth: () => void;
  onThreadPanelResizeStart: (
    event: React.PointerEvent<HTMLButtonElement>,
  ) => void;
  threadPanelWidthPx: number;
  useSplitAuxiliaryPane: boolean;
  transparentChrome?: boolean;
};

export function ChannelManagementAuxiliaryPanel({
  activeChannel,
  canResetThreadPanelWidth,
  currentPubkey,
  isSinglePanelView,
  onChannelManagementDeleted,
  onCloseChannelManagement,
  onResetThreadPanelWidth,
  onThreadPanelResizeStart,
  threadPanelWidthPx,
  useSplitAuxiliaryPane,
  transparentChrome = false,
}: ChannelManagementAuxiliaryPanelProps) {
  const panel = (
    <ChannelManagementSheet
      animateSplitEnter={isSinglePanelView && !useSplitAuxiliaryPane}
      channel={activeChannel}
      currentPubkey={currentPubkey}
      layout={useSplitAuxiliaryPane || isSinglePanelView ? "split" : "overlay"}
      onDeleted={onChannelManagementDeleted}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) {
          onCloseChannelManagement?.();
        }
      }}
      open={true}
      transparentChrome={transparentChrome}
    />
  );

  if (!useSplitAuxiliaryPane) {
    return panel;
  }

  return (
    <RightAuxiliaryPane
      canResetWidth={canResetThreadPanelWidth}
      onResetWidth={onResetThreadPanelWidth}
      onResizeStart={onThreadPanelResizeStart}
      testId="channel-management-auxiliary-pane"
      widthPx={threadPanelWidthPx}
    >
      {panel}
    </RightAuxiliaryPane>
  );
}
