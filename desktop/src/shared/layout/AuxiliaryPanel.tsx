import * as React from "react";

import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { THREAD_PANEL_MIN_WIDTH_PX } from "@/shared/hooks/useThreadPanelWidth";
import {
  AuxiliaryPanelContext,
  type AuxiliaryPanelLayout,
} from "@/shared/layout/auxiliaryPanelContext";
import { getAuxiliaryPanelMode } from "@/shared/layout/AuxiliaryPanelHeader";
import { cn } from "@/shared/lib/cn";
import {
  OverlayPanelBackdrop,
  PANEL_ENTER_BASE_CLASS,
  PANEL_OVERLAY_CLASS,
} from "@/shared/ui/OverlayPanelBackdrop";

export type {
  AuxiliaryPanelContextValue,
  AuxiliaryPanelLayout,
} from "@/shared/layout/auxiliaryPanelContext";
export { useAuxiliaryPanel } from "@/shared/layout/auxiliaryPanelContext";

type AuxiliaryPanelProps = {
  canResetWidth?: boolean;
  children: React.ReactNode;
  className?: string;
  footer?: React.ReactNode;
  header?: React.ReactNode;
  isSinglePanelView?: boolean;
  layout?: AuxiliaryPanelLayout;
  onClose: () => void;
  onResetWidth?: () => void;
  onResizeStart?: React.PointerEventHandler<HTMLButtonElement>;
  resizeHandleAriaLabel?: string;
  resizeHandleTestId?: string;
  siblings?: React.ReactNode;
  /** When false, standalone width uses `widthPx` without min-width clamp. */
  splitPaneClamp?: boolean;
  testId?: string;
  transparentChrome?: boolean;
  widthPx: number;
};

/** Right-side auxiliary panel shell for split and standalone overlay layouts. */
export function AuxiliaryPanel({
  canResetWidth,
  children,
  className,
  footer,
  header,
  isSinglePanelView = false,
  layout = "standalone",
  onClose,
  onResetWidth,
  onResizeStart,
  resizeHandleAriaLabel = "Resize panel",
  resizeHandleTestId,
  siblings,
  splitPaneClamp = true,
  testId,
  transparentChrome = false,
  widthPx,
}: AuxiliaryPanelProps) {
  const isOverlay = useIsThreadPanelOverlay();
  const isFloatingOverlay = isOverlay && !isSinglePanelView;
  const isSplitLayout = layout === "split";
  const mode = getAuxiliaryPanelMode(isSplitLayout, isFloatingOverlay);

  const contextValue = React.useMemo(
    () => ({
      isFloatingOverlay,
      isOverlay,
      isSinglePanelView,
      isSplitLayout,
      layout,
      mode,
      onClose,
      transparentChrome,
      widthPx,
    }),
    [
      isFloatingOverlay,
      isOverlay,
      isSinglePanelView,
      isSplitLayout,
      layout,
      mode,
      onClose,
      transparentChrome,
      widthPx,
    ],
  );

  const panelWidth = isSinglePanelView
    ? "100%"
    : splitPaneClamp
      ? `min(${widthPx}px, calc(100% - ${THREAD_PANEL_MIN_WIDTH_PX}px))`
      : `${widthPx}px`;

  const resizeHandle =
    !isSplitLayout &&
    !isOverlay &&
    !isSinglePanelView &&
    onResizeStart != null ? (
      <button
        aria-label={resizeHandleAriaLabel}
        className="peer/profile-resize group/profile-resize absolute inset-y-0 left-0 z-40 w-3 -translate-x-1/2 cursor-col-resize"
        data-testid={resizeHandleTestId}
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
    ) : null;

  if (isSplitLayout) {
    return (
      <AuxiliaryPanelContext.Provider value={contextValue}>
        <div className={cn("flex min-h-0 flex-1 flex-col", className)}>
          {header}
          {children}
          {footer}
        </div>
        {siblings}
      </AuxiliaryPanelContext.Provider>
    );
  }

  return (
    <AuxiliaryPanelContext.Provider value={contextValue}>
      {isFloatingOverlay ? <OverlayPanelBackdrop onClose={onClose} /> : null}
      <aside
        className={cn(
          PANEL_ENTER_BASE_CLASS,
          isSinglePanelView && "border-l-0",
          isFloatingOverlay && PANEL_OVERLAY_CLASS,
        )}
        data-testid={testId}
        style={{ width: panelWidth }}
      >
        {resizeHandle}
        {header}
        {children}
        {footer}
      </aside>
      {siblings}
    </AuxiliaryPanelContext.Provider>
  );
}
