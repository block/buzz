import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import {
  KIND_FORUM_COMMENT,
  KIND_FORUM_POST,
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
} from "@/shared/constants/kinds";

import { extractPlaygroundCards } from "./card.ts";
import {
  addPlaygroundTab,
  ensureBrowserForSid,
  getLiveBrowserForSid,
  getPlaygroundBrowser,
  getPlaygroundStore,
  listPlaygroundSessions,
  markPlaygroundUpdate,
  notePlaygroundCard,
  subscribePlayground,
  switchPlaygroundTab,
} from "./sessions.ts";
import { isAllowedPlaygroundUrl } from "./url.ts";
import {
  rebindBrowserAgentGrantToTab,
  subscribeBrowserAgentSetViewport,
  subscribeBrowserAgentSwitchTab,
  syncBrowserAgentTabs,
  syncBrowserAgentViewport,
} from "@/features/browser-agent/lib/api";
import {
  applyAgentPlaygroundViewport,
  getPlaygroundViewport,
  subscribePlaygroundViewport,
} from "./playgroundViewport";
import { mainTabSid } from "./browserGroups.ts";
import { browserWebviewLabel } from "@/features/browser-agent/lib/labels";
import {
  PLAYGROUND_DOM_POLL_INTERVAL_MS,
  PLAYGROUND_DOM_PROBE_SCRIPT,
  PLAYGROUND_POLL_INTERVAL_MS,
  nextPlaygroundDomUpdate,
} from "./updates.ts";
import {
  getActiveEmbeddedWindow,
  subscribeEmbeddedWindows,
  getEmbeddedWindowsStore,
} from "@/features/popout/lib/embeddedWindows";
import { currentPopoutPayload } from "@/features/popout/lib/popoutWindow";
import {
  getPopoutWindows,
  playgroundSidsHostedInOsPopouts,
  subscribePopoutWindows,
} from "@/features/popout/lib/popoutWindows";
import {
  currentWindowLabel,
  evalPlaygroundWebview,
  playgroundWebviewDomHash,
  pollPlaygroundWebview,
  playgroundKeeperParkBounds,
  showPlaygroundWebview,
  subscribePlaygroundNewTab,
} from "./webview.ts";

/** Legacy off-screen placeholder; prefer {@link playgroundKeeperParkBounds}. */
const KEEPER_BOUNDS = { x: -64, y: -64, width: 64, height: 64 };
const domBaselines = new Map<string, string>();

export function usePlaygroundRuntime() {
  usePlaygroundFenceWatcher();
  usePlaygroundWebviewKeeper();
  usePlaygroundUpdatePolling();
  usePlaygroundNewTabListener();
  usePlaygroundTabSwitchListener();
  usePlaygroundViewportListener();
  usePlaygroundViewportSync();
}


/** Last new-tab emit key — decidePolicy + createWebView can both fire once. */
let lastNewTabDedupeKey = "";
let lastNewTabDedupeAt = 0;
const NEW_TAB_DEDUPE_MS = 400;

/**
 * Apply a playground-webview-new-tab payload: sibling tab in opener group.
 * Returns the new session sid, or null when ignored (bad url / no group / dedupe).
 */
export function handlePlaygroundNewTabRequest(payload: {
  openerSid?: string;
  openerLabel?: string;
  url?: string;
}): string | null {
  const openerSid = payload.openerSid?.trim() ?? "";
  const url = payload.url?.trim() ?? "";
  if (!openerSid || !url) return null;
  // Secondary playground tabs require https (same as main sessions).
  // about:blank / empty window.open() cannot carry a later location with
  // NewWindowResponse::Deny — skip rather than create a dead tab.
  if (!isAllowedPlaygroundUrl(url)) {
    console.warn(
      "buzz-playground: ignoring new-tab request (https required)",
      url,
    );
    return null;
  }
  const dedupeKey = `${openerSid}|${url}`;
  const now = Date.now();
  if (
    dedupeKey === lastNewTabDedupeKey &&
    now - lastNewTabDedupeAt < NEW_TAB_DEDUPE_MS
  ) {
    return null;
  }
  lastNewTabDedupeKey = dedupeKey;
  lastNewTabDedupeAt = now;

  const browser =
    getLiveBrowserForSid(openerSid) ?? ensureBrowserForSid(openerSid);
  if (!browser) {
    console.warn(
      "buzz-playground: new-tab opener has no browser group",
      openerSid,
    );
    return null;
  }
  // Preserve overlay host (side-panel slide-out) — do not pass preferSidePanel.
  const session = addPlaygroundTab({
    browserId: browser.browserId,
    url,
  });
  if (!session) return null;
  const windowLabel = currentWindowLabel();
  void rebindBrowserAgentGrantToTab({
    fromSurfaceId: openerSid,
    toSurfaceId: session.sid,
    toWebviewLabel: browserWebviewLabel({
      surface: "playground",
      surfaceId: session.sid,
      windowLabel,
    }),
    fromWebviewLabel:
      payload.openerLabel ||
      browserWebviewLabel({
        surface: "playground",
        surfaceId: openerSid,
        windowLabel,
      }),
  }).catch(() => undefined);
  void syncAndAnnounceBrowserTabs(browser.browserId, {
    kind: "tab_opened",
    surfaceId: session.sid,
    openerSurfaceId: openerSid,
    url,
  }).catch(() => undefined);
  return session.sid;
}

/**
 * MVP B: Rust denies the native popup and emits playground-webview-new-tab.
 * Create a sibling tab in the opener's browser group, focus it, rebind grant.
 */
function usePlaygroundNewTabListener() {
  React.useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void subscribePlaygroundNewTab((payload) => {
      if (disposed) return;
      handlePlaygroundNewTabRequest(payload);
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
        return;
      }
      stop = unlisten;
    });
    return () => {
      disposed = true;
      stop?.();
    };
  }, []);
}



/**
 * Agent MCP browser_set_viewport → Desktop emits browser-agent-set-viewport.
 * Apply via the same playground viewport store Stage UI uses.
 */
function usePlaygroundViewportListener() {
  React.useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void subscribeBrowserAgentSetViewport((payload) => {
      if (disposed) return;
      const sid = payload.surfaceId?.trim();
      if (!sid) return;
      const result = applyAgentPlaygroundViewport(sid, payload);
      if (!result.ok) {
        console.warn("buzz: browser_set_viewport rejected", result.error);
        return;
      }
      void syncPlaygroundViewportToAgent(sid).catch(() => undefined);
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
        return;
      }
      stop = unlisten;
    });
    return () => {
      disposed = true;
      stop?.();
    };
  }, []);
}

/** Mirror live viewport snapshots under the grant dir for MCP browser_get_viewport. */
export async function syncPlaygroundViewportToAgent(sid: string) {
  const snap = getPlaygroundViewport(sid);
  await syncBrowserAgentViewport({
    surfaceId: sid,
    mode: snap.mode,
    width: snap.width,
    height: snap.height,
    scalePercent: snap.scalePercent,
    deviceId: snap.deviceId,
    orientation: snap.orientation,
  });
}

function usePlaygroundViewportSync() {
  React.useEffect(() => {
    let cancelled = false;
    const push = () => {
      if (cancelled) return;
      for (const session of listPlaygroundSessions()) {
        void syncPlaygroundViewportToAgent(session.sid).catch(() => undefined);
      }
    };
    push();
    const unsub = subscribePlaygroundViewport(push);
    return () => {
      cancelled = true;
      unsub();
    };
  }, []);
}

/**
 * Agent MCP browser_switch_tab → Desktop emits browser-agent-switch-tab.
 * Focus that surfaceId in its group and rebind the grant.
 */
function usePlaygroundTabSwitchListener() {
  React.useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void subscribeBrowserAgentSwitchTab((payload) => {
      if (disposed) return;
      const target = payload.surfaceId?.trim();
      if (!target) return;
      const browser = getLiveBrowserForSid(target);
      if (!browser) return;
      const fromSid = browser.activeTabSid;
      switchPlaygroundTab(target);
      if (fromSid !== target) {
        const windowLabel = currentWindowLabel();
        void rebindBrowserAgentGrantToTab({
          fromSurfaceId: fromSid,
          toSurfaceId: target,
          toWebviewLabel: browserWebviewLabel({
            surface: "playground",
            surfaceId: target,
            windowLabel,
          }),
          fromWebviewLabel: browserWebviewLabel({
            surface: "playground",
            surfaceId: fromSid,
            windowLabel,
          }),
        }).catch(() => undefined);
      }
      void syncAndAnnounceBrowserTabs(browser.browserId, {
        kind: "tab_switched",
        surfaceId: target,
      }).catch(() => undefined);
    }).then((unlisten) => {
      if (disposed) {
        unlisten();
        return;
      }
      stop = unlisten;
    });
    return () => {
      disposed = true;
      stop?.();
    };
  }, []);
}

/** Mirror browser-group tabs for MCP + optional observe announce. */
export async function syncAndAnnounceBrowserTabs(
  browserId: string,
  event?: {
    kind: "tab_opened" | "tab_switched";
    surfaceId: string;
    openerSurfaceId?: string;
    url?: string;
  },
) {
  const store = getPlaygroundStore();
  const browser = getPlaygroundBrowser(browserId);
  if (!browser) return;
  const tabs = browser.tabSids.map((sid) => {
    const session = store.sessions.get(sid);
    return {
      surfaceId: sid,
      url: session?.url ?? "",
      title: session?.name ?? "",
      isMain: sid === mainTabSid(browser),
    };
  });
  await syncBrowserAgentTabs({
    browserId: browser.browserId,
    mainTabSid: mainTabSid(browser),
    activeTabSid: browser.activeTabSid,
    tabs,
    event: event ?? null,
  });
}

function usePlaygroundFenceWatcher() {
  React.useEffect(() => {
    let cancelled = false;
    let dispose: (() => void) | undefined;
    void relayClient
      .subscribeLive(
        {
          kinds: [
            KIND_STREAM_MESSAGE,
            KIND_STREAM_MESSAGE_V2,
            KIND_FORUM_POST,
            KIND_FORUM_COMMENT,
          ],
          limit: 0,
        },
        (event) => {
          if (typeof event.content !== "string") return;
          for (const card of extractPlaygroundCards(event.content)) {
            notePlaygroundCard(card);
          }
        },
      )
      .then((unsub) => {
        if (cancelled) {
          unsub();
          return;
        }
        dispose = unsub;
      })
      .catch(() => {
        // Browser/unit hosts have no live relay.
      });
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, []);
}

function activeEmbedPlaygroundSid(): string | null {
  return getActiveEmbeddedWindow()?.payload.playground?.sid ?? null;
}

function usePlaygroundWebviewKeeper() {
  const { sessions, overlaySid } = React.useSyncExternalStore(
    subscribePlayground,
    getPlaygroundStore,
    getPlaygroundStore,
  );
  React.useSyncExternalStore(
    subscribeEmbeddedWindows,
    getEmbeddedWindowsStore,
    getEmbeddedWindowsStore,
  );
  React.useSyncExternalStore(
    subscribePopoutWindows,
    getPopoutWindows,
    getPopoutWindows,
  );
  const popout = currentPopoutPayload();
  const embedSid = activeEmbedPlaygroundSid();
  const osHostedKey = [...playgroundSidsHostedInOsPopouts(getPopoutWindows())]
    .sort()
    .join(",");

  React.useEffect(() => {
    // Pop-out windows do not run the main-window keeper.
    if (popout) return;
    const osHosted = new Set(osHostedKey ? osHostedKey.split(",") : []);
    for (const session of sessions.values()) {
      if (overlaySid === session.sid) continue;
      if (embedSid === session.sid) continue;
      // OS split/playground hosts the one live WKWebView (reparented). Do not
      // keeper-show or hide from main — that would blank Drive mid-flight.
      if (osHosted.has(session.sid)) {
        continue;
      }
      // Keep last full stage size while parked. Shrinking to 64×64 collapsed
      // the live page viewport and broke Drive clicks ("no element") until the
      // user re-opened the card. Native show also refuses to apply keeper
      // park bounds on an existing WKWebView.
      void showPlaygroundWebview({
        sid: session.sid,
        url: session.url,
        bounds: playgroundKeeperParkBounds(session.sid, KEEPER_BOUNDS),
        visible: false,
      }).then(() =>
        evalPlaygroundWebview(session.sid, PLAYGROUND_DOM_PROBE_SCRIPT),
      );
    }
  }, [sessions, overlaySid, popout, embedSid, osHostedKey]);
}

function usePlaygroundUpdatePolling() {
  React.useEffect(() => {
    if (currentPopoutPayload()) return;
    let cancelled = false;
    const poll = async () => {
      if (cancelled) return;
      const { overlaySid } = getPlaygroundStore();
      for (const session of listPlaygroundSessions()) {
        if (overlaySid === session.sid) continue;
        if (activeEmbedPlaygroundSid() === session.sid) continue;
        try {
          await evalPlaygroundWebview(session.sid, PLAYGROUND_DOM_PROBE_SCRIPT);
          const raw = await playgroundWebviewDomHash(session.sid, session.url);
          const next = nextPlaygroundDomUpdate(
            domBaselines.get(session.sid) ?? null,
            { hash: raw || null, ready: Boolean(raw) },
          );
          if (next.baseline) domBaselines.set(session.sid, next.baseline);
          if (next.changed) markPlaygroundUpdate(session.sid);
        } catch {
          // SPA hash is best-effort while the hidden webview is warming.
        }
      }
    };
    const pollHttp = async () => {
      if (cancelled) return;
      const { overlaySid } = getPlaygroundStore();
      for (const session of listPlaygroundSessions()) {
        if (overlaySid === session.sid) continue;
        if (activeEmbedPlaygroundSid() === session.sid) continue;
        try {
          const http = await pollPlaygroundWebview(session.sid, session.url);
          if (http.changed) markPlaygroundUpdate(session.sid);
        } catch {
          // Parked HTTP poll is best-effort.
        }
      }
    };
    void poll();
    void pollHttp();
    const httpTimer = window.setInterval(() => {
      void pollHttp();
    }, PLAYGROUND_POLL_INTERVAL_MS);
    const domTimer = window.setInterval(() => {
      void poll();
    }, PLAYGROUND_DOM_POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(httpTimer);
      window.clearInterval(domTimer);
    };
  }, []);
}
