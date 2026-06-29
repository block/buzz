import type * as React from "react";

import { AuxiliaryPanel } from "@/shared/layout/AuxiliaryPanel";
import { AuxiliaryPanelHeader } from "@/shared/layout/AuxiliaryPanelHeader";

type UserProfilePanelFrameProps = {
  addAgentToChannelDialog: React.ReactNode;
  canResetWidth?: boolean;
  editAgentDialog: React.ReactNode;
  headerActions: React.ReactNode;
  headerLeftContent: React.ReactNode;
  isOverlay: boolean;
  isSinglePanelView: boolean;
  isSplitLayout: boolean;
  onClose: () => void;
  onResetWidth?: () => void;
  onResizeStart?: React.PointerEventHandler<HTMLButtonElement>;
  personaDialogs: React.ReactNode;
  profileBody: React.ReactNode;
  splitPaneClamp: boolean;
  widthPx: number;
  transparentChrome?: boolean;
};

export function UserProfilePanelFrame({
  addAgentToChannelDialog,
  canResetWidth,
  editAgentDialog,
  headerActions,
  headerLeftContent,
  isOverlay,
  isSinglePanelView,
  isSplitLayout,
  onClose,
  onResetWidth,
  onResizeStart,
  personaDialogs,
  profileBody,
  splitPaneClamp,
  widthPx,
  transparentChrome = false,
}: UserProfilePanelFrameProps) {
  return (
    <AuxiliaryPanel
      canResetWidth={canResetWidth}
      isSinglePanelView={isSinglePanelView}
      layout={isSplitLayout ? "split" : "standalone"}
      onClose={onClose}
      onResetWidth={onResetWidth}
      onResizeStart={onResizeStart}
      resizeHandleAriaLabel="Resize profile panel"
      resizeHandleTestId="user-profile-resize-handle"
      siblings={
        <>
          {editAgentDialog}
          {addAgentToChannelDialog}
          {personaDialogs}
        </>
      }
      splitPaneClamp={splitPaneClamp}
      testId="user-profile-panel"
      transparentChrome={transparentChrome}
      widthPx={widthPx}
      header={
        <AuxiliaryPanelHeader
          backdrop={!isSplitLayout && !isOverlay}
          inset={!isSplitLayout ? "wide" : "default"}
          resizeBorder={!isSinglePanelView && !isOverlay && !isSplitLayout}
          surface={isSinglePanelView ? "transparent" : "default"}
        >
          {headerLeftContent}
          {headerActions}
        </AuxiliaryPanelHeader>
      }
    >
      {profileBody}
    </AuxiliaryPanel>
  );
}
