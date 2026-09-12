import * as React from "react";

/** A relay-hosted markdown attachment the in-app viewer can open. */
export type MarkdownDocTarget = {
  /** Raw relay `/media/` URL of the attachment (pre-proxy-rewrite). */
  url: string;
  /** Human-readable filename from the message's imeta `filename` field. */
  filename: string;
  /**
   * The card element that invoked the open, for focus restoration when the
   * panel closes. The URL alone cannot identify the opener: the same
   * attachment can appear in several messages, giving several cards one URL.
   */
  opener?: HTMLElement | null;
};

/**
 * Open-in-viewer callback for markdown document attachments.
 *
 * Provided by surfaces that host a markdown-doc auxiliary panel (the channel
 * screen). Where no provider exists — project PR/issue bodies, read-only
 * previews — the context is `null` and `FileCard` keeps its default
 * download-only behavior, so the viewer affordance can never appear
 * somewhere it has no panel to open.
 */
const MarkdownDocViewerContext = React.createContext<
  ((doc: MarkdownDocTarget) => void) | null
>(null);

export const MarkdownDocViewerProvider = MarkdownDocViewerContext.Provider;

/** The active surface's open-in-viewer callback, or null when unhosted. */
export function useMarkdownDocViewer():
  | ((doc: MarkdownDocTarget) => void)
  | null {
  return React.useContext(MarkdownDocViewerContext);
}
