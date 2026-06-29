import type * as React from "react";

import { THREAD_PANEL_MIN_WIDTH_PX } from "@/shared/hooks/useThreadPanelWidth";
import { AuxiliaryPanelHeader } from "@/shared/layout/AuxiliaryPanelHeader";
import { cn } from "@/shared/lib/cn";
import {
  OverlayPanelBackdrop,
  PANEL_ENTER_BASE_CLASS,
  PANEL_OVERLAY_CLASS,
  PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS,
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
  if (isSplitLayout) {
    return (
      <>
        <div className="flex min-h-0 flex-1 flex-col">
          <AuxiliaryPanelHeader transparent={transparentChrome}>
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

        {!isOverlay ? (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-x-0 top-0 z-40 h-13 bg-background/80 backdrop-blur-md supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55"
          />
        ) : null}

        <div
          className={cn(
            "flex cursor-default select-none items-center",
            isSinglePanelView
              ? `relative ${PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS} -mb-13 min-h-13 shrink-0 gap-2.5 bg-transparent px-4 py-2 sm:pl-6 sm:pr-3`
              : isOverlay
                ? "relative z-50 min-h-13 shrink-0 gap-3 bg-background/80 px-5 py-2 backdrop-blur-md supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55"
                : "absolute inset-x-0 top-0 z-50 min-h-13 gap-3 bg-transparent px-3 py-2 after:absolute after:bottom-0 after:-left-px after:top-0 after:w-px after:bg-border/45 after:transition-colors peer-hover/profile-resize:after:bg-border/80 peer-focus-visible/profile-resize:after:bg-border/80",
          )}
          data-tauri-drag-region
        >
          {headerLeftContent}
          {headerActions}
        </div>

        {profileBody}
      </aside>
      {editAgentDialog}
      {addAgentToChannelDialog}
      {personaDialogs}
    </>
  );
}
