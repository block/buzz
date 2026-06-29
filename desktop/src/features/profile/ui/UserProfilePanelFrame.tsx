import type * as React from "react";

import { THREAD_PANEL_MIN_WIDTH_PX } from "@/shared/hooks/useThreadPanelWidth";
import {
  AuxiliaryPanelHeader,
  getAuxiliaryPanelMode,
} from "@/shared/layout/AuxiliaryPanelHeader";
import { cn } from "@/shared/lib/cn";
import {
  OverlayPanelBackdrop,
  PANEL_ENTER_BASE_CLASS,
  PANEL_OVERLAY_CLASS,
} from "@/shared/ui/OverlayPanelBackdrop";

type UserProfilePanelFrameProps = {
  addAgentToChannelDialog: React.ReactNode;
  canResetWidth?: boolean;
  editAgentDialog: React.ReactNode;
  headerActions: React.ReactNode;
  headerLeftContent: React.ReactNode;
  isFloatingOverlay: boolean;
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
  isFloatingOverlay,
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
  const mode = getAuxiliaryPanelMode(isSplitLayout, isFloatingOverlay);

  if (mode === "docked") {
    return (
      <>
        <div className="flex min-h-0 flex-1 flex-col">
          <AuxiliaryPanelHeader mode={mode} transparent={transparentChrome}>
            {headerLeftContent}
            {headerActions}
          </AuxiliaryPanelHeader>
          {profileBody}
        </div>
        {editAgentDialog}
        {addAgentToChannelDialog}
        {personaDialogs}
      </>
    );
  }

  return (
    <>
      {isFloatingOverlay && <OverlayPanelBackdrop onClose={onClose} />}
      <aside
        className={cn(
          PANEL_ENTER_BASE_CLASS,
          isSinglePanelView && "border-l-0",
          isFloatingOverlay && PANEL_OVERLAY_CLASS,
        )}
        data-testid="user-profile-panel"
        style={{
          width: isSinglePanelView
            ? "100%"
            : splitPaneClamp
              ? `min(${widthPx}px, calc(100% - ${THREAD_PANEL_MIN_WIDTH_PX}px))`
              : `${widthPx}px`,
        }}
      >
        {!isOverlay && !isSinglePanelView && onResizeStart && (
          <button
            aria-label="Resize profile panel"
            className="peer/profile-resize group/profile-resize absolute inset-y-0 left-0 z-40 w-3 -translate-x-1/2 cursor-col-resize"
            data-testid="user-profile-resize-handle"
            onDoubleClick={canResetWidth ? onResetWidth : undefined}
            onPointerDown={onResizeStart}
            title={
              canResetWidth
                ? "Drag to resize. Double-click to reset width."
                : "Drag to resize."
            }
            type="button"
          >
            <span className="absolute bottom-0 left-1/2 top-10 w-px -translate-x-1/2 bg-transparent transition-colors group-hover/profile-resize:bg-border/80 group-focus-visible/profile-resize:bg-border/80" />
          </button>
        )}

        <AuxiliaryPanelHeader
          backdrop={!isOverlay}
          inset="wide"
          mode={mode}
          resizeBorder={!isSinglePanelView && !isOverlay}
          surface={isSinglePanelView ? "transparent" : "default"}
        >
          {headerLeftContent}
          {headerActions}
        </AuxiliaryPanelHeader>

        {profileBody}
      </aside>
      {editAgentDialog}
      {addAgentToChannelDialog}
      {personaDialogs}
    </>
  );
}
