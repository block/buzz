import { parkPlaygroundHost } from "@/features/playground/lib/sessions";
import {
  closePinWebview,
  hidePinWebview,
} from "@/features/pinned-sites/lib/pinWebview";

/** Stable pin-webview id for the channel/thread link slide-out. */
export const LINK_SIDE_PANEL_PIN_ID = "hula-link-side-panel";

export type LinkSidePanelViewportMode = "desktop" | "responsive" | "mobile";

export type LinkSidePanelState = {
  expanded: boolean;
  title: string;
  url: string;
  viewportMode: LinkSidePanelViewportMode;
  /** Native pin webview id. Defaults to {@link LINK_SIDE_PANEL_PIN_ID}. */
  pinId: string;
  /**
   * When true, closing the panel hides the native webview instead of
   * destroying it so a conversation playground pin can resume later.
   */
  keepAlive: boolean;
};

export type OpenLinkSidePanelOptions = {
  title?: string;
  pinId?: string;
  keepAlive?: boolean;
};

type Store = {
  panel: LinkSidePanelState | null;
};

const store: Store = {
  panel: null,
};

const listeners = new Set<() => void>();

let cachedSnapshot: Store = { panel: null };

function snapshot(): Store {
  return { panel: store.panel };
}

function emit() {
  cachedSnapshot = snapshot();
  for (const listener of listeners) listener();
}

function titleFromUrl(url: string): string {
  try {
    const parsed = new URL(url);
    return parsed.hostname.replace(/^www\./, "") || "Link";
  } catch {
    return "Link";
  }
}

function teardownPanelWebview(panel: LinkSidePanelState): void {
  if (panel.keepAlive) {
    void hidePinWebview(panel.pinId);
  } else {
    void closePinWebview(panel.pinId);
  }
}

/** True for http(s) URLs that belong in the in-app link slide-out. */
export function isLinkSidePanelUrl(url: string): boolean {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}

export function subscribeLinkSidePanel(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getLinkSidePanelStore(): Store {
  return cachedSnapshot;
}

export function getLinkSidePanel(): LinkSidePanelState | null {
  return cachedSnapshot.panel;
}

/**
 * Pin a URL into the Projects-style right slide-out beside the active
 * channel/thread. Parks playground overlay/embed hosts first so native
 * children cannot strand over the conversation (same class of teardown as
 * PRs #53/#54).
 *
 * Pass `keepAlive` + a stable `pinId` for conversation playground pins so
 * closing the panel parks the webview instead of destroying it.
 */
export function openLinkSidePanel(
  url: string,
  options?: OpenLinkSidePanelOptions,
): boolean {
  const trimmed = url.trim();
  if (!isLinkSidePanelUrl(trimmed)) return false;
  parkPlaygroundHost();

  const nextPinId = options?.pinId?.trim() || LINK_SIDE_PANEL_PIN_ID;
  const nextKeepAlive = Boolean(options?.keepAlive);
  const title = options?.title?.trim() || titleFromUrl(trimmed);

  const previous = store.panel;
  if (
    previous &&
    (previous.pinId !== nextPinId || previous.keepAlive !== nextKeepAlive)
  ) {
    // Switching targets: park or destroy the outgoing webview first.
    teardownPanelWebview(previous);
  }

  store.panel = {
    expanded: false,
    title,
    url: trimmed,
    pinId: nextPinId,
    keepAlive: nextKeepAlive,
    viewportMode: previous?.viewportMode ?? "desktop",
  };
  emit();
  return true;
}

export function setLinkSidePanelExpanded(expanded: boolean): void {
  if (!store.panel) return;
  if (store.panel.expanded === expanded) return;
  store.panel = { ...store.panel, expanded };
  emit();
}

export function toggleLinkSidePanelExpanded(): void {
  if (!store.panel) return;
  setLinkSidePanelExpanded(!store.panel.expanded);
}

export function setLinkSidePanelViewportMode(
  viewportMode: LinkSidePanelViewportMode,
): void {
  if (!store.panel) return;
  if (store.panel.viewportMode === viewportMode) return;
  store.panel = { ...store.panel, viewportMode };
  emit();
}

/**
 * Close the link slide-out. Keep-alive playground pins hide their native
 * webview; ordinary link opens destroy {@link LINK_SIDE_PANEL_PIN_ID}.
 */
export function closeLinkSidePanel(): void {
  if (!store.panel) {
    void closePinWebview(LINK_SIDE_PANEL_PIN_ID);
    return;
  }
  const panel = store.panel;
  store.panel = null;
  emit();
  teardownPanelWebview(panel);
}

/**
 * Destroy a keep-alive (or ordinary) link webview and clear the slide-out when
 * it was showing that pin — used when unpinning a conversation playground.
 */
export function destroyLinkSidePanelIfPin(pinId: string): void {
  if (store.panel?.pinId === pinId) {
    store.panel = null;
    emit();
  }
  void closePinWebview(pinId);
}

/** Test helper — clear listeners and state without native teardown. */
export function resetLinkSidePanelStore(): void {
  store.panel = null;
  cachedSnapshot = { panel: null };
}
