import { MarkdownDocPanel } from "@/features/channels/ui/MarkdownDocPanel";
import type { MarkdownDocTarget } from "@/shared/ui/markdown/markdownDocViewerContext";

type MarkdownDocAuxiliaryPanelProps = {
  doc: MarkdownDocTarget;
  /** Render chrome for the full focus drawer rather than a narrow pane. */
  isFocusDrawer?: boolean;
  isSinglePanelView: boolean;
  onClose: () => void;
  useSplitAuxiliaryPane: boolean;
  widthPx: number;
};

/**
 * Assembles the markdown-document auxiliary pane for ChannelPane's pane
 * chain. Split out of ChannelPane.tsx to keep it under the per-file line
 * cap.
 *
 * This is the chain's lowest-priority pane: a higher-priority pane opened
 * afterwards (thread, activity, profile) shows immediately, and the document
 * reappears when it closes. Opening a document clears competitors in the
 * screen-level handler, so it is never dead on arrival.
 */
export function MarkdownDocAuxiliaryPanel({
  doc,
  isFocusDrawer = false,
  isSinglePanelView,
  onClose,
  useSplitAuxiliaryPane,
  widthPx,
}: MarkdownDocAuxiliaryPanelProps) {
  return (
    // Keyed by URL so opening a different document resets the Preview/Code
    // toggle instead of inheriting the previous document's.
    <MarkdownDocPanel
      key={doc.url}
      filename={doc.filename}
      isFocusMode={isFocusDrawer}
      isSinglePanelView={
        isFocusDrawer || useSplitAuxiliaryPane ? false : isSinglePanelView
      }
      layout={useSplitAuxiliaryPane && !isFocusDrawer ? "split" : "standalone"}
      onClose={onClose}
      transparentChrome={useSplitAuxiliaryPane && !isFocusDrawer}
      url={doc.url}
      widthPx={widthPx}
    />
  );
}
