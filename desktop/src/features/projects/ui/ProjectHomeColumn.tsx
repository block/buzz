import type * as React from "react";

import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";
import {
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
  onResetWidth,
  onResizeStart,
  testId,
  title,
  widthPx,
}: {
  bodyClassName?: string;
  canResetWidth: boolean;
  children: React.ReactNode;
  onResetWidth: () => void;
  onResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  testId: string;
  title: string;
  widthPx: number;
}) {
  return (
    <RightAuxiliaryPane
      canResetWidth={canResetWidth}
      className="bg-sidebar text-sidebar-foreground"
      constrainToAvailableSpace={false}
      detached
      onResetWidth={onResetWidth}
      onResizeStart={onResizeStart}
      testId={testId}
      widthPx={widthPx}
    >
      <div className="relative z-30 flex h-full min-h-0 flex-1 flex-col overflow-hidden bg-sidebar">
        <AuxiliaryPanelHeader mode="panel" transparent>
          <AuxiliaryPanelHeaderGroup>
            <div className="-ml-1 min-w-0 flex-1">
              <AuxiliaryPanelHeaderTitleBlock title={title} />
            </div>
          </AuxiliaryPanelHeaderGroup>
        </AuxiliaryPanelHeader>
        <AuxiliaryPanelBody
          className={cn("min-h-0 flex-1 overflow-hidden", bodyClassName)}
          mode="panel"
        >
          {children}
        </AuxiliaryPanelBody>
      </div>
    </RightAuxiliaryPane>
  );
}
