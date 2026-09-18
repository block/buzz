import type * as React from "react";

import {
  AuxiliaryPanel,
  AuxiliaryPanelBody,
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderActions,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelTitle,
} from "@/shared/layout/AuxiliaryPanel";
import { cn } from "@/shared/lib/cn";

export type IdleAuxiliaryHeaderControls = {
  actions?: React.ReactNode;
  backLabel?: string;
  onBack?: () => void;
};

export function IdleAuxiliaryPanel({
  bodyClassName,
  canResetWidth,
  children,
  headerControls,
  isFocusDrawer = false,
  isSinglePanelView,
  onClose,
  onResetWidth,
  onResizeStart,
  title,
  useSplitAuxiliaryPane,
  widthPx,
}: {
  /** Override body scroll/padding — link/pin webviews need overflow-hidden fill. */
  bodyClassName?: string;
  canResetWidth: boolean;
  children: React.ReactNode;
  headerControls?: IdleAuxiliaryHeaderControls;
  isFocusDrawer?: boolean;
  isSinglePanelView: boolean;
  onClose: () => void;
  onResetWidth: () => void;
  onResizeStart: React.PointerEventHandler<HTMLButtonElement>;
  title: string;
  useSplitAuxiliaryPane: boolean;
  widthPx: number;
}) {
  const split = useSplitAuxiliaryPane && !isFocusDrawer;
  return (
    <AuxiliaryPanel
      canResetWidth={canResetWidth}
      enterMotion={!isFocusDrawer}
      isSinglePanelView={
        isFocusDrawer ? true : split ? false : isSinglePanelView
      }
      layout={split ? "split" : "standalone"}
      onClose={onClose}
      onResetWidth={onResetWidth}
      onResizeStart={onResizeStart}
      resizeHandleAriaLabel="Resize panel"
      resizeHandleTestId="idle-auxiliary-resize-handle"
      testId="idle-auxiliary-panel"
      transparentChrome={split}
      widthPx={widthPx}
      header={
        <AuxiliaryPanelHeader backdrop={isFocusDrawer} transparent={split}>
          <AuxiliaryPanelHeaderGroup
            backButtonAriaLabel={headerControls?.backLabel}
            backButtonTestId="idle-auxiliary-back"
            onBack={headerControls?.onBack}
          >
            <AuxiliaryPanelTitle>{title}</AuxiliaryPanelTitle>
          </AuxiliaryPanelHeaderGroup>
          {headerControls?.actions ? (
            <AuxiliaryPanelHeaderActions>
              {headerControls.actions}
            </AuxiliaryPanelHeaderActions>
          ) : null}
        </AuxiliaryPanelHeader>
      }
    >
      <AuxiliaryPanelBody
        className={cn(
          "overflow-y-auto overflow-x-hidden overscroll-contain px-4 pb-8",
          bodyClassName,
        )}
      >
        {children}
      </AuxiliaryPanelBody>
    </AuxiliaryPanel>
  );
}
