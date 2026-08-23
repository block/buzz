import type * as React from "react";

import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";
import {
  AuxiliaryPanel,
  AuxiliaryPanelBody,
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelHeaderTitleBlock,
} from "@/shared/layout/AuxiliaryPanel";
import { cn } from "@/shared/lib/cn";

export function ProjectHomeColumn({
  bodyClassName,
  canResetWidth,
  children,
  onClose,
  onResetWidth,
  onResizeStart,
  testId,
  title,
  widthPx,
}: {
  bodyClassName?: string;
  canResetWidth: boolean;
  children: React.ReactNode;
  onClose: () => void;
  onResetWidth: () => void;
  onResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  testId: string;
  title: string;
  widthPx: number;
}) {
  return (
    <RightAuxiliaryPane
      canResetWidth={canResetWidth}
      constrainToAvailableSpace={false}
      detached
      onResetWidth={onResetWidth}
      onResizeStart={onResizeStart}
      testId={testId}
      widthPx={widthPx}
    >
      <div className="relative z-30 flex h-full min-h-0 flex-1 flex-col overflow-hidden rounded-2xl bg-background">
        <AuxiliaryPanel
          layout="split"
          onClose={onClose}
          transparentChrome
          widthPx={widthPx}
          header={
            <AuxiliaryPanelHeader transparent>
              <AuxiliaryPanelHeaderGroup>
                <AuxiliaryPanelHeaderTitleBlock title={title} />
              </AuxiliaryPanelHeaderGroup>
            </AuxiliaryPanelHeader>
          }
        >
          <AuxiliaryPanelBody
            className={cn("min-h-0 flex-1 overflow-hidden", bodyClassName)}
          >
            {children}
          </AuxiliaryPanelBody>
        </AuxiliaryPanel>
      </div>
    </RightAuxiliaryPane>
  );
}
