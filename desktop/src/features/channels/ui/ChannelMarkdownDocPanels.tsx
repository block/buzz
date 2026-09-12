import type * as React from "react";

import { MarkdownDocAuxiliaryPanel } from "@/features/channels/ui/MarkdownDocAuxiliaryPanel";
import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";
import type { MarkdownDocTarget } from "@/shared/ui/markdown/markdownDocViewerContext";

type ChannelMarkdownDocPanelProps = {
  canResetWidth: boolean;
  doc: MarkdownDocTarget;
  onClose: () => void;
  onResetWidth: () => void;
  onResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  stacked?: boolean;
  widthPx: number;
};

/** Document pane shown beside, or stacked over, a still-mounted thread. */
export function ChannelMarkdownDocPanel({
  canResetWidth,
  doc,
  onClose,
  onResetWidth,
  onResizeStart,
  stacked = false,
  widthPx,
}: ChannelMarkdownDocPanelProps) {
  return (
    <RightAuxiliaryPane
      canResetWidth={canResetWidth}
      className={stacked ? "absolute inset-y-0 right-0 z-41" : undefined}
      onResetWidth={onResetWidth}
      onResizeStart={onResizeStart}
      testId={
        stacked ? "markdown-doc-stacked-panel" : "markdown-doc-third-panel"
      }
      widthPx={widthPx}
    >
      <MarkdownDocAuxiliaryPanel
        doc={doc}
        isSinglePanelView={false}
        onClose={onClose}
        useSplitAuxiliaryPane
        widthPx={widthPx}
      />
    </RightAuxiliaryPane>
  );
}
