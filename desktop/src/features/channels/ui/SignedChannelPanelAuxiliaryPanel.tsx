import type * as React from "react";

import { SignedChannelPanel } from "@/features/channels/ui/SignedChannelPanel";
import type { SignedChannelPanelState } from "@/features/channels/ui/signedChannelPanelTypes";
import { AuxiliaryPanel } from "@/shared/layout/AuxiliaryPanel";
import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";

type SignedChannelPanelAuxiliaryPanelProps = {
  canResetPanelWidth: boolean;
  channelName: string;
  isSinglePanelView: boolean;
  onClose: () => void;
  onOpenSourceEvent?: (eventId: string) => void;
  onResetPanelWidth: () => void;
  onPanelResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  panelWidthPx: number;
  state: SignedChannelPanelState;
  useSplitAuxiliaryPane: boolean;
};

export function SignedChannelPanelAuxiliaryPanel({
  canResetPanelWidth,
  channelName,
  isSinglePanelView,
  onClose,
  onOpenSourceEvent,
  onResetPanelWidth,
  onPanelResizeStart,
  panelWidthPx,
  state,
  useSplitAuxiliaryPane,
}: SignedChannelPanelAuxiliaryPanelProps) {
  const content = (
    <SignedChannelPanel
      channelName={channelName}
      mode={
        useSplitAuxiliaryPane
          ? "docked"
          : isSinglePanelView
            ? "single-panel"
            : "panel"
      }
      onOpenSourceEvent={onOpenSourceEvent}
      state={state}
    />
  );

  if (useSplitAuxiliaryPane) {
    return (
      <RightAuxiliaryPane
        canResetWidth={canResetPanelWidth}
        onResetWidth={onResetPanelWidth}
        onResizeStart={onPanelResizeStart}
        testId="signed-channel-panel-auxiliary-pane"
        widthPx={panelWidthPx}
      >
        <AuxiliaryPanel
          layout="split"
          onClose={onClose}
          testId="signed-channel-panel-shell"
          widthPx={panelWidthPx}
        >
          {content}
        </AuxiliaryPanel>
      </RightAuxiliaryPane>
    );
  }

  return (
    <AuxiliaryPanel
      isSinglePanelView={isSinglePanelView}
      layout="standalone"
      onClose={onClose}
      testId="signed-channel-panel-shell"
      widthPx={panelWidthPx}
    >
      {content}
    </AuxiliaryPanel>
  );
}
