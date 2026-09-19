import type * as React from "react";
import { AnimatePresence } from "motion/react";

import { ChannelMarkdownDocPanel } from "@/features/channels/ui/ChannelMarkdownDocPanels";
import type { MarkdownDocTarget } from "@/shared/ui/markdown/markdownDocViewerContext";

type Props = {
  canResetThreadPanelWidth: boolean;
  idleAuxiliarySurface: React.ReactNode;
  onCloseMarkdownDoc?: () => void;
  onResetThreadPanelWidth: () => void;
  onThreadPanelResizeStart: (
    event: React.PointerEvent<HTMLButtonElement>,
  ) => void;
  openMarkdownDoc: MarkdownDocTarget | null;
  showIdleAuxiliaryOverThread: boolean;
  showMarkdownBesideThread: boolean;
  threadPanelWidthPx: number;
  threadSurface: { markExitComplete: () => void };
  useStackedMarkdownPanel: boolean;
};

/** Presence boundaries for responsive Markdown panes around an open thread. */
export function ChannelMarkdownDocSurfaces(props: Props) {
  const renderPanel = (stacked = false) =>
    props.openMarkdownDoc && props.onCloseMarkdownDoc ? (
      <ChannelMarkdownDocPanel
        canResetWidth={props.canResetThreadPanelWidth}
        doc={props.openMarkdownDoc}
        onClose={props.onCloseMarkdownDoc}
        onResetWidth={props.onResetThreadPanelWidth}
        onResizeStart={props.onThreadPanelResizeStart}
        stacked={stacked}
        widthPx={props.threadPanelWidthPx}
      />
    ) : null;
  return (
    <>
      <AnimatePresence>
        {props.showMarkdownBesideThread ? renderPanel() : null}
      </AnimatePresence>
      <AnimatePresence onExitComplete={props.threadSurface.markExitComplete}>
        {props.useStackedMarkdownPanel
          ? renderPanel(true)
          : props.showIdleAuxiliaryOverThread
            ? props.idleAuxiliarySurface
            : null}
      </AnimatePresence>
    </>
  );
}
