import * as React from "react";

/** User preference for how link previews are presented. */
export type LinkPreviewStyle = "compact" | "rich";

export const LINK_PREVIEW_STYLE_STORAGE_KEY =
  "buzz.appearance.linkPreviewStyle";
export const DEFAULT_LINK_PREVIEW_STYLE: LinkPreviewStyle = "compact";

const listeners = new Set<() => void>();
let linkPreviewStyle = readStoredLinkPreviewStyle();

export function parseLinkPreviewStyle(
  value: string | null | undefined,
): LinkPreviewStyle {
  return value === "rich" || value === "compact"
    ? value
    : DEFAULT_LINK_PREVIEW_STYLE;
}

function readStoredLinkPreviewStyle(): LinkPreviewStyle {
  try {
    return parseLinkPreviewStyle(
      globalThis.localStorage?.getItem(LINK_PREVIEW_STYLE_STORAGE_KEY),
    );
  } catch {
    return DEFAULT_LINK_PREVIEW_STYLE;
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getLinkPreviewStyle(): LinkPreviewStyle {
  return linkPreviewStyle;
}

export function setLinkPreviewStyle(style: LinkPreviewStyle): void {
  linkPreviewStyle = style;
  try {
    globalThis.localStorage?.setItem(LINK_PREVIEW_STYLE_STORAGE_KEY, style);
  } catch {
    // Persistence is best-effort; the in-memory preference still applies.
  }
  for (const listener of listeners) listener();
}

export function useLinkPreviewStyle(): LinkPreviewStyle {
  return React.useSyncExternalStore(
    subscribe,
    getLinkPreviewStyle,
    () => DEFAULT_LINK_PREVIEW_STYLE,
  );
}
