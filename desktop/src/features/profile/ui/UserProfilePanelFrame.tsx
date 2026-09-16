import type * as React from "react";

import {
  AUXILIARY_PANEL_DEFAULT_SURFACE_CLASS,
  AuxiliaryPanel,
  AuxiliaryPanelHeader,
} from "@/shared/layout/AuxiliaryPanel";
import { cn } from "@/shared/lib/cn";

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
  stickyChromeActive: boolean;
  stickyChromeEnabled: boolean;
  stickyChromeHeight: number;
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
  stickyChromeActive,
  stickyChromeEnabled,
  stickyChromeHeight,
  widthPx,
  transparentChrome = false,
}: UserProfilePanelFrameProps) {
  return (
    <AuxiliaryPanel
      canResetWidth={canResetWidth}
      className="relative"
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
        <>
          <div
            aria-hidden="true"
            className={cn(
              "pointer-events-none absolute inset-x-0 top-0 z-30 opacity-0 transition-opacity duration-150 ease-out motion-reduce:transition-none",
              AUXILIARY_PANEL_DEFAULT_SURFACE_CLASS,
              stickyChromeActive && "opacity-100",
            )}
            data-active={stickyChromeActive ? "true" : "false"}
            data-testid="user-profile-sticky-chrome-surface"
            style={{ height: `${stickyChromeHeight}px` }}
          />
          <AuxiliaryPanelHeader
            backdrop={false}
            data-testid="user-profile-panel-header"
            inset={!isSplitLayout ? "wide" : "default"}
            resizeBorder={!isSinglePanelView && !isOverlay && !isSplitLayout}
            surface={stickyChromeEnabled ? "transparent" : "default"}
            transparent={stickyChromeEnabled}
          >
            {headerLeftContent}
            {headerActions}
          </AuxiliaryPanelHeader>
        </>
      }
    >
      {profileBody}
    </AuxiliaryPanel>
  );
}
