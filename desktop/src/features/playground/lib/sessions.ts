import { getStorageItem, setStorageItem } from "@/shared/lib/safeStorage";
import { hideAllPinWebviews } from "@/features/pinned-sites/lib/pinWebview";
import { notifyPinWebviewRestore } from "@/shared/lib/nativeWebviewModalPark";
import {
  dismissEmbeddedWindow,
  getActiveEmbeddedWindow,
  registerEmbedOpenHandler,
} from "@/features/popout/lib/embeddedWindows";
import {
  closePopoutWindow,
  getPopoutWindows,
  playgroundSidsHostedInOsPopouts,
  popoutKindFromLabel,
} from "@/features/popout/lib/popoutWindows";
import { readPopoutPayload } from "@/features/popout/lib/popoutWindow";
import type { PlaygroundCard } from "./types";
import {
  clearPlaygroundViewport,
  resetPlaygroundViewports,
} from "./playgroundViewport";
import {
  addTabToBrowser,
  isMainBrowserTab,
  createOneTabBrowser,
  findBrowserById,
  findBrowserByTabSid,
  isValidBrowser,
  migrateSessionsToBrowsers,
  removeTabFromBrowser,
  setActiveBrowserTab,
  type PlaygroundBrowser,
} from "./browserGroups";
import {
  configureConversationPlaygroundPinsScope,
  resetConversationPlaygroundPins,
  unpinPlaygroundSessionEverywhere,
} from "./conversationPins";
import { playgroundSessionsMatchingCard } from "./updates";
import {
  closeAllPlaygroundWebviews,
  closePlaygroundWebview,
  hideAllPlaygroundWebviews,
  hidePlaygroundWebview,
} from "./webview";

export const PLAYGROUND_STORAGE_VERSION = "v1";

export type { PlaygroundBrowser } from "./browserGroups";

export type PlaygroundSession = {
  sid: string;
  name: string;
  url: string;
  pin?: string;
  stack?: string;
  expires?: string;
  hasUpdate?: boolean;
};

/**
 * Where the active overlaySid is hosted in the main window.
 * - side-panel: RHS idle-auxiliary / FocusThreadDrawer slide-out (card Open /
 *   pin Open / Browsers Open / Watch / Drive) with Agent chrome
 * - window: idle/cleared default only — NEVER an active in-app canvas host.
 *   Product lock: browsers may only live as (1) RHS slide-out, (2) split with
 *   a thread, (3) detached OS/pop-out. Full-canvas PlaygroundOverlay inset-0
 *   via overlayHost "window" is banned.
 * Not persisted.
 */
export type PlaygroundOverlayHost = "window" | "side-panel";

export type ShowPlaygroundOptions = {
  /**
   * Host in the channel RHS idle-auxiliary slide-out (same host as
   * openLinkSidePanel) with PlaygroundChrome + grants — not left dock,
   * not windowed inset-0 cover.
   *
   * Product lock: `false` is dead-ended — in-app Open always uses side-panel.
   * Detach uses openPopoutWindow (real OS window), not overlayHost "window".
   */
  preferSidePanel?: boolean;
};

type PlaygroundStore = {
  sessions: Map<string, PlaygroundSession>;
  browsers: Map<string, PlaygroundBrowser>;
  overlaySid: string | null;
  overlayHost: PlaygroundOverlayHost;
};

const store: PlaygroundStore = {
  sessions: new Map(),
  browsers: new Map(),
  overlaySid: null,
  overlayHost: "window",
};

/**
 * Update overlay host only when the caller passes options.
 * Omitted options preserve the current host so tab switch / window.open sibling
 * tabs stay in the same slide-out (side-panel) and do not promote away.
 *
 * Product lock: any explicit options → side-panel. preferSidePanel: false
 * (legacy full-canvas window overlay) is dead-ended.
 */
function noteOverlayHost(options?: ShowPlaygroundOptions) {
  if (options == null) return;
  store.overlayHost = "side-panel";
}

let currentScope: string | null = null;
let cachedSnapshot: PlaygroundStore = {
  sessions: new Map(),
  browsers: new Map(),
  overlaySid: null,
  overlayHost: "window",
};
const listeners = new Set<() => void>();

function snapshot(): PlaygroundStore {
  return {
    sessions: new Map(store.sessions),
    browsers: new Map(store.browsers),
    overlaySid: store.overlaySid,
    overlayHost: store.overlayHost,
  };
}

function emit() {
  cachedSnapshot = snapshot();
  for (const listener of listeners) listener();
}

function persist() {
  if (!currentScope) return;
  setStorageItem(
    currentScope,
    JSON.stringify({
      sessions: [...store.sessions.values()],
      browsers: [...store.browsers.values()],
      overlaySid: store.overlaySid,
    }),
  );
}

function cardToSession(card: PlaygroundCard): PlaygroundSession {
  return {
    sid: card.sid,
    name: card.name,
    url: card.url,
    ...(card.pin ? { pin: card.pin } : {}),
    ...(card.stack ? { stack: card.stack } : {}),
    ...(card.expires != null ? { expires: String(card.expires) } : {}),
    hasUpdate: false,
  };
}

export function playgroundStorageKey(pubkey: string, relayUrl: string): string {
  return `buzz-playground.${PLAYGROUND_STORAGE_VERSION}:${pubkey}:${relayUrl}`;
}

export function subscribePlayground(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getPlaygroundStore(): PlaygroundStore {
  return cachedSnapshot;
}

export function listPlaygroundSessions(): PlaygroundSession[] {
  return [...cachedSnapshot.sessions.values()];
}

export function getActivePlaygroundSid(): string | null {
  return cachedSnapshot.overlaySid;
}

export function configurePlaygroundScope(pubkey: string, relayUrl: string) {
  const key = playgroundStorageKey(pubkey, relayUrl);
  if (currentScope === key) return;
  currentScope = key;
  store.sessions.clear();
  store.browsers.clear();
  store.overlaySid = null;
  store.overlayHost = "window";
  const raw = getStorageItem(key);
  if (raw) {
    try {
      const saved = JSON.parse(raw) as {
        sessions?: PlaygroundSession[];
        browsers?: PlaygroundBrowser[];
        overlaySid?: string | null;
      };
      for (const session of saved.sessions ?? []) {
        if (session?.sid && session.name && session.url) {
          store.sessions.set(session.sid, {
            ...session,
            hasUpdate: Boolean(session.hasUpdate),
          });
        }
      }
      const savedBrowsers = (saved.browsers ?? []).filter(isValidBrowser);
      for (const browser of migrateSessionsToBrowsers(
        [...store.sessions.keys()],
        savedBrowsers,
      )) {
        store.browsers.set(browser.browserId, browser);
      }
    } catch {
      // Ignore a corrupt blob; the in-memory store stays empty.
    }
  } else {
    for (const browser of migrateSessionsToBrowsers([...store.sessions.keys()])) {
      store.browsers.set(browser.browserId, browser);
    }
  }
  // Channel/thread pins share this identity; prune orphans vs live session sids.
  configureConversationPlaygroundPinsScope(
    pubkey,
    relayUrl,
    store.sessions.keys(),
  );
  emit();
}

export function addPlaygroundSession(
  card: PlaygroundCard,
  options?: ShowPlaygroundOptions,
): PlaygroundSession {
  // New session without options → side-panel (product lock). Tab APIs omit to preserve.
  noteOverlayHost(options ?? { preferSidePanel: true });
  const session = cardToSession(card);
  store.sessions.set(session.sid, session);
  // One-tab browser group (browserId may equal sid initially).
  const browser = createOneTabBrowser(session.sid);
  store.browsers.set(browser.browserId, browser);
  store.overlaySid = session.sid;
  persist();
  emit();
  dismissEmbeddedWindow();
  void hideAllPinWebviews();
  return session;
}

function createTabSessionId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `tab-${Date.now().toString(36)}`;
}

/**
 * Open a new tab in an existing browser group (window.open / + / target=_blank).
 * Focuses the new tab. Caller supplies url (+ optional name).
 */
export function addPlaygroundTab(input: {
  browserId: string;
  url: string;
  name?: string;
  options?: ShowPlaygroundOptions;
}): PlaygroundSession | null {
  const browser = store.browsers.get(input.browserId);
  if (!browser) return null;
  const url = input.url.trim();
  if (!url) return null;
  noteOverlayHost(input.options);
  const sid = createTabSessionId();
  const name =
    (input.name ?? "").trim() ||
    (() => {
      try {
        return new URL(url).hostname || "Tab";
      } catch {
        return "Tab";
      }
    })();
  const session: PlaygroundSession = {
    sid,
    name: name.slice(0, 80),
    url,
    hasUpdate: false,
  };
  store.sessions.set(sid, session);
  store.browsers.set(input.browserId, addTabToBrowser(browser, sid));
  store.overlaySid = sid;
  persist();
  emit();
  dismissEmbeddedWindow();
  void hideAllPinWebviews();
  return session;
}

/** Focus a tab in its browser group (shows that webview; hides siblings via stage). */
export function switchPlaygroundTab(sid: string, options?: ShowPlaygroundOptions) {
  if (!store.sessions.has(sid)) return;
  const browser = findBrowserByTabSid([...store.browsers.values()], sid);
  if (!browser) {
    showPlaygroundSession(sid, options);
    return;
  }
  noteOverlayHost(options);
  store.browsers.set(
    browser.browserId,
    setActiveBrowserTab(browser, sid),
  );
  const session = store.sessions.get(sid);
  if (session) {
    store.sessions.set(sid, { ...session, hasUpdate: false });
  }
  store.overlaySid = sid;
  persist();
  emit();
  dismissEmbeddedWindow();
  void hideAllPinWebviews();
}

export function listPlaygroundBrowsers(): PlaygroundBrowser[] {
  return [...cachedSnapshot.browsers.values()];
}

export function getBrowserForSid(sid: string): PlaygroundBrowser | null {
  return findBrowserByTabSid([...cachedSnapshot.browsers.values()], sid);
}

/** Live store lookup (event handlers — avoid stale snapshot). */
export function getLiveBrowserForSid(sid: string): PlaygroundBrowser | null {
  return findBrowserByTabSid([...store.browsers.values()], sid);
}

/** Ensure opener sid has a browser group (migrate orphan sessions). */
export function ensureBrowserForSid(sid: string): PlaygroundBrowser | null {
  const existing = getLiveBrowserForSid(sid);
  if (existing) return existing;
  if (!store.sessions.has(sid)) return null;
  const browser = createOneTabBrowser(sid);
  store.browsers.set(browser.browserId, browser);
  persist();
  emit();
  return browser;
}

export function getPlaygroundBrowser(browserId: string): PlaygroundBrowser | null {
  return findBrowserById([...cachedSnapshot.browsers.values()], browserId);
}

/** Dispose every tab in a browser group (Browsers Remove). */
export function disposePlaygroundBrowser(browserId: string) {
  const browser = store.browsers.get(browserId);
  if (!browser) return;
  const sids = [...browser.tabSids];
  store.browsers.delete(browserId);
  for (const sid of sids) {
    disposePlaygroundSessionOnly(sid);
  }
  persist();
  emit();
  void hideAllPinWebviews();
  notifyPinRestore();
}

/**
 * Close one secondary tab. The main/primary tab (tabSids[0]) cannot be
 * dismissed here — use disposePlaygroundBrowser / Browsers Remove.
 */
export function closePlaygroundTab(sid: string) {
  const browser = findBrowserByTabSid([...store.browsers.values()], sid);
  if (!browser) {
    disposePlayground(sid);
    return;
  }
  // Main tab has no X; refuse programmatic close while the group exists.
  if (isMainBrowserTab(browser, sid)) {
    return;
  }
  const next = removeTabFromBrowser(browser, sid);
  if (!next) {
    store.browsers.delete(browser.browserId);
    disposePlaygroundSessionOnly(sid);
    persist();
    emit();
    void hideAllPinWebviews();
    notifyPinRestore();
    return;
  }
  store.browsers.set(browser.browserId, next);
  // Switch the live overlay to the surviving tab *before* disposing the closed
  // sid. disposePlaygroundSessionOnly clears overlayHost to "window" whenever
  // overlaySid === sid; doing dispose first promoted side-panel → window and
  // mounted PlaygroundOverlay (absolute inset-0) over the whole channel canvas.
  if (store.overlaySid === sid) {
    store.overlaySid = next.activeTabSid;
  }
  disposePlaygroundSessionOnly(sid);
  if (store.overlaySid == null) {
    store.overlaySid = next.activeTabSid;
  }
  persist();
  emit();
  void hideAllPinWebviews();
  notifyPinRestore();
}

/** Tear down one session/webview without touching browser-group maps. */
function disposePlaygroundSessionOnly(sid: string) {
  const embed = getActiveEmbeddedWindowSafe();
  if (embed?.payload.playground?.sid === sid) {
    dismissEmbeddedWindow();
  }
  // Hook: drop channel/thread pins bound to this session (Browsers Remove).
  unpinPlaygroundSessionEverywhere(sid);
  store.sessions.delete(sid);
  clearPlaygroundViewport(sid);
  if (store.overlaySid === sid) {
    store.overlaySid = null;
    store.overlayHost = "window";
  }
  void closePlaygroundWebview(sid);
}


/** Close OS playground/split pop-outs that host this sid so pin Open / RHS
 * stage reparents the one live WKWebView instead of fighting a sibling stage. */
function releaseOsPopoutsHostingSid(sid: string): void {
  const trimmed = sid.trim();
  if (!trimmed) return;
  if (!playgroundSidsHostedInOsPopouts(getPopoutWindows()).has(trimmed)) {
    return;
  }
  for (const row of getPopoutWindows()) {
    const kind = popoutKindFromLabel(row.label);
    if (kind !== "playground" && kind !== "split") continue;
    const hosted = readPopoutPayload(row.label)?.playground?.sid;
    if (hosted !== trimmed) continue;
    void closePopoutWindow(row.label);
  }
}

/** True when this sid already has a left-menu playground row. */
export function hasPlaygroundSession(sid: string): boolean {
  return store.sessions.has(sid);
}

export function showPlaygroundSession(
  sid: string,
  options?: ShowPlaygroundOptions,
) {
  if (!store.sessions.has(sid)) return;
  // Omitted options → side-panel (product lock). Tab switch/add omit to preserve.
  noteOverlayHost(options ?? { preferSidePanel: true });
  const session = store.sessions.get(sid);
  if (session) {
    store.sessions.set(sid, { ...session, hasUpdate: false });
  }
  const browser = findBrowserByTabSid([...store.browsers.values()], sid);
  if (browser) {
    store.browsers.set(
      browser.browserId,
      setActiveBrowserTab(browser, sid),
    );
  }
  store.overlaySid = sid;
  persist();
  emit();
  dismissEmbeddedWindow();
  // Claim on main/RHS: release OS hosts so Destroyed reparents the live
  // WKWebView to main (parked) instead of leaving a competing stage.
  if (options?.preferSidePanel) {
    releaseOsPopoutsHostingSid(sid);
  }
  void hideAllPinWebviews();
}

export function markPlaygroundUpdate(sid: string) {
  const session = store.sessions.get(sid);
  if (!session || session.hasUpdate) return;
  if (store.overlaySid === sid) return;
  store.sessions.set(sid, { ...session, hasUpdate: true });
  persist();
  emit();
}

export function notePlaygroundCard(card: PlaygroundCard) {
  for (const session of playgroundSessionsMatchingCard(
    card,
    store.sessions.values(),
  )) {
    markPlaygroundUpdate(session.sid);
  }
}

/**
 * Park overlay and in-main embed, then run a left-nav destination. Safe
 * when nothing is showing. Does not dispose the session row.
 */
export function parkPlaygroundThen(select: () => void): () => void {
  return () => {
    parkPlaygroundHost();
    select();
  };
}

/**
 * Park whatever is hosting a playground in the main window: overlay and/or
 * active embed. Always hides the native webview for the parked sid so
 * navigate-away cannot leave an orphan WKWebView when overlaySid was already
 * null (embed-only / split-embed path).
 */
export function parkPlaygroundHost(): void {
  store.overlayHost = "window";
  const overlaySid = store.overlaySid;
  const embed = getActiveEmbeddedWindowSafe();
  const embedSid = embed?.payload.playground?.sid ?? null;
  if (store.overlaySid != null) {
    store.overlaySid = null;
    persist();
    emit();
  }
  dismissEmbeddedWindow();
  const sid = overlaySid ?? embedSid;
  // Prefer the known sid, then hide every playground on this window so an
  // in-flight show that races dismiss cannot leave an orphan topmost WKWebView.
  if (sid) void hidePlaygroundWebview(sid);
  void hideAllPlaygroundWebviews();
  void hideAllPinWebviews();
  notifyPinRestore();
}

/**
 * Dismiss the visible playground host (overlay and/or in-main embed).
 * Same teardown as navigate-away park — chrome X must not clear React state
 * while leaving a native child painted.
 */
export function dismissPlayground() {
  parkPlaygroundHost();
}

/** Clear only the overlay sid — used when opening an embed so we do not
 * immediately park the embed we just activated. */
export function dismissOverlayPlayground() {
  const sid = store.overlaySid;
  if (sid == null) return;
  store.overlaySid = null;
  store.overlayHost = "window";
  persist();
  emit();
  void hidePlaygroundWebview(sid);
  notifyPinRestore();
}

export function disposePlayground(sid: string) {
  const browser = findBrowserByTabSid([...store.browsers.values()], sid);
  if (browser) {
    // Closing a tab through dispose removes it from the group; last tab drops the group.
    const next = removeTabFromBrowser(browser, sid);
    if (!next) {
      store.browsers.delete(browser.browserId);
    } else {
      store.browsers.set(browser.browserId, next);
      if (store.overlaySid === sid) {
        store.overlaySid = next.activeTabSid;
      }
    }
  }
  disposePlaygroundSessionOnly(sid);
  // If group still has an active tab, restore overlay to it.
  if (browser) {
    const remaining = store.browsers.get(browser.browserId);
    if (remaining && store.overlaySid == null) {
      store.overlaySid = remaining.activeTabSid;
    }
  }
  persist();
  emit();
  void hideAllPinWebviews();
  notifyPinRestore();
}

function getActiveEmbeddedWindowSafe() {
  try {
    return getActiveEmbeddedWindow();
  } catch {
    return null;
  }
}

function notifyPinRestore() {
  // Defer while a blocking modal park is held so dismiss cannot restore a
  // pin/link webview underneath an open Dialog/Sheet.
  notifyPinWebviewRestore();
}

/** True when the active session should render in the RHS idle-auxiliary panel. */
export function isPlaygroundSidePanelHost(): boolean {
  return (
    cachedSnapshot.overlaySid != null &&
    cachedSnapshot.overlayHost === "side-panel"
  );
}

export function getPlaygroundOverlayHost(): PlaygroundOverlayHost {
  return cachedSnapshot.overlayHost;
}

export function resetPlaygroundState() {
  currentScope = null;
  store.sessions.clear();
  store.browsers.clear();
  store.overlaySid = null;
  store.overlayHost = "window";
  resetPlaygroundViewports();
  resetConversationPlaygroundPins();
  emit();
  void hideAllPlaygroundWebviews();
  void closeAllPlaygroundWebviews();
}

if (import.meta.env.MODE === "test") {
  (
    globalThis as { __BUZZ_PLAYGROUND_TEST__?: unknown }
  ).__BUZZ_PLAYGROUND_TEST__ = {
    addPlaygroundSession,
    addPlaygroundTab,
    switchPlaygroundTab,
    closePlaygroundTab,
    disposePlaygroundBrowser,
    hasPlaygroundSession,
    parkPlaygroundThen,
    parkPlaygroundHost,
    showPlaygroundSession,
    isPlaygroundSidePanelHost,
    getPlaygroundOverlayHost,
    dismissPlayground,
    dismissOverlayPlayground,
    disposePlayground,
    listPlaygroundSessions,
    listPlaygroundBrowsers,
    getBrowserForSid,
    getPlaygroundBrowser,
    getActivePlaygroundSid,
    configurePlaygroundScope,
    resetPlaygroundState,
    playgroundStorageKey,
    markPlaygroundUpdate,
    notePlaygroundCard,
  };
}

registerEmbedOpenHandler(dismissOverlayPlayground);
