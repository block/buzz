import * as React from "react";

import type { RelayEvent } from "@/shared/api/types";
import { SignedChannelPanel } from "@/features/channels/ui/SignedChannelPanel";
import type { TimelineMessage } from "@/features/messages/types";
import { composeSignedChannelPanelState } from "@/features/channels/ui/composeSignedChannelPanel";
import { AuxiliaryPanel } from "@/shared/layout/AuxiliaryPanel";
import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";

type SignedChannelPanelAuxiliaryPanelProps = {
  canResetPanelWidth: boolean;
  channelId: string;
  channelName: string;
  events: readonly RelayEvent[];
  isLoading: boolean;
  isSinglePanelView: boolean;
  onClose: () => void;
  onOpenThread: (message: TimelineMessage) => void;
  onResetPanelWidth: () => void;
  onPanelResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  panelWidthPx: number;
  sourceMessages: TimelineMessage[];
  useSplitAuxiliaryPane: boolean;
};

export function SignedChannelPanelAuxiliaryPanel({
  canResetPanelWidth,
  channelId,
  channelName,
  events,
  isLoading,
  isSinglePanelView,
  onClose,
  onOpenThread,
  onResetPanelWidth,
  onPanelResizeStart,
  panelWidthPx,
  sourceMessages,
  useSplitAuxiliaryPane,
}: SignedChannelPanelAuxiliaryPanelProps) {
  const state = React.useMemo(
    () =>
      isLoading
        ? { kind: "loading" as const }
        : composeSignedChannelPanelState(channelId, events),
    [channelId, events, isLoading],
  );
  const onOpenSourceEvent = React.useCallback(
    (eventId: string) => {
      const sourceMessage = sourceMessages.find(
        (message) => message.id === eventId,
      );
      if (sourceMessage) onOpenThread(sourceMessage);
    },
    [onOpenThread, sourceMessages],
  );
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
