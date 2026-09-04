import type * as React from "react";

import { MarkdownDocAuxiliaryPanel } from "@/features/channels/ui/MarkdownDocAuxiliaryPanel";
import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";
import type { MarkdownDocTarget } from "@/shared/ui/markdown/markdownDocViewerContext";

type HomeMarkdownDocPanelProps = {
  canResetWidth: boolean;
  className?: string;
  doc: MarkdownDocTarget;
  onClose: () => void;
  onResetWidth: () => void;
  onResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  testId: string;
  widthPx: number;
};

/** Inbox-local Markdown viewer, rendered either as a third pane or a stack. */
export function HomeMarkdownDocPanel({
  canResetWidth,
  className,
  doc,
  onClose,
  onResetWidth,
  onResizeStart,
  testId,
  widthPx,
}: HomeMarkdownDocPanelProps) {
  return (
    <RightAuxiliaryPane
      canResetWidth={canResetWidth}
      className={className}
      constrainToAvailableSpace={false}
      onResetWidth={onResetWidth}
      onResizeStart={onResizeStart}
      testId={testId}
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
