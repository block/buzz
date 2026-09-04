import type * as React from "react";

import type { MarkdownDocTarget } from "@/shared/ui/markdown/markdownDocViewerContext";
import { HomeMarkdownDocPanel } from "./HomeMarkdownDocPanel";

type Props = {
  besideDetail: boolean;
  canResetWidth: boolean;
  doc: MarkdownDocTarget | null;
  onClose: () => void;
  onResetWidth: () => void;
  onResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  showDetail: boolean;
  widthPx: number;
};

/** Responsive Inbox document surfaces: third pane when roomy, stack otherwise. */
export function HomeMarkdownDocSurfaces(props: Props) {
  if (!props.doc) return null;
  return props.besideDetail ? (
    <HomeMarkdownDocPanel
      canResetWidth={props.canResetWidth}
      doc={props.doc}
      onClose={props.onClose}
      onResetWidth={props.onResetWidth}
      onResizeStart={props.onResizeStart}
      testId="home-markdown-doc-panel"
      widthPx={props.widthPx}
    />
  ) : props.showDetail ? (
    <HomeMarkdownDocPanel
      canResetWidth={props.canResetWidth}
      className="absolute inset-y-0 right-0 z-41"
      doc={props.doc}
      onClose={props.onClose}
      onResetWidth={props.onResetWidth}
      onResizeStart={props.onResizeStart}
      testId="home-markdown-doc-stacked-panel"
      widthPx={props.widthPx}
    />
  ) : null;
}
