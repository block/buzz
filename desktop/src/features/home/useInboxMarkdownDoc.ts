import * as React from "react";

import type { MarkdownDocTarget } from "@/shared/ui/markdown/markdownDocViewerContext";
import { AUXILIARY_PANEL_MIN_WIDTH_PX } from "@/shared/layout/AuxiliaryPanel";

/** Owns Inbox-local document state and its responsive third-pane threshold. */
export function useInboxMarkdownDoc({
  conversationId,
  homeWidthPx,
  inboxListWidthPx,
  panelWidthPx,
}: {
  conversationId: string | null;
  homeWidthPx: number;
  inboxListWidthPx: number;
  panelWidthPx: number;
}) {
  const [state, setState] = React.useState<{
    conversationId: string | null;
    doc: MarkdownDocTarget | null;
  }>({ conversationId, doc: null });
  const doc = state.conversationId === conversationId ? state.doc : null;
  const open = React.useCallback(
    (next: MarkdownDocTarget | null) => setState({ conversationId, doc: next }),
    [conversationId],
  );
  const besideDetail =
    doc !== null &&
    homeWidthPx >=
      inboxListWidthPx + panelWidthPx + AUXILIARY_PANEL_MIN_WIDTH_PX;
  return {
    besideDetail,
    close: React.useCallback(() => open(null), [open]),
    doc,
    open,
  };
}
