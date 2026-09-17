import { parkPlaygroundHost } from "@/features/playground/lib/sessions";
import { closePinWebview } from "@/features/pinned-sites/lib/pinWebview";

/** Stable pin-webview id for the channel/thread link slide-out. */
export const LINK_SIDE_PANEL_PIN_ID = "hula-link-side-panel";

export type LinkSidePanelState = {
  expanded: boolean;
  title: string;
  url: string;
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
 */
export function openLinkSidePanel(url: string): boolean {
  const trimmed = url.trim();
  if (!isLinkSidePanelUrl(trimmed)) return false;
  parkPlaygroundHost();
  store.panel = {
    expanded: false,
    title: titleFromUrl(trimmed),
    url: trimmed,
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

/**
 * Close the link slide-out and destroy its native pin webview so nothing
 * remains painted over the channel.
 */
export function closeLinkSidePanel(): void {
  if (!store.panel) {
    void closePinWebview(LINK_SIDE_PANEL_PIN_ID);
    return;
  }
  store.panel = null;
  emit();
  void closePinWebview(LINK_SIDE_PANEL_PIN_ID);
}

/** Test helper — clear listeners and state without native teardown. */
export function resetLinkSidePanelStore(): void {
  store.panel = null;
  cachedSnapshot = { panel: null };
}
