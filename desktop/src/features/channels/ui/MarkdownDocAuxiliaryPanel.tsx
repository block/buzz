import { MarkdownDocPanel } from "@/features/channels/ui/MarkdownDocPanel";
import type { MarkdownDocTarget } from "@/shared/ui/markdown/markdownDocViewerContext";

type MarkdownDocAuxiliaryPanelProps = {
  doc: MarkdownDocTarget;
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
      isSinglePanelView={useSplitAuxiliaryPane ? false : isSinglePanelView}
      layout={useSplitAuxiliaryPane ? "split" : "standalone"}
      onClose={onClose}
      transparentChrome={useSplitAuxiliaryPane}
      url={doc.url}
      widthPx={widthPx}
    />
  );
}
