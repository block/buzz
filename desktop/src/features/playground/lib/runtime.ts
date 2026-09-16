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
  getPlaygroundStore,
  listPlaygroundSessions,
  markPlaygroundUpdate,
  notePlaygroundCard,
  subscribePlayground,
} from "./sessions.ts";
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
  evalPlaygroundWebview,
  hidePlaygroundWebview,
  playgroundWebviewDomHash,
  pollPlaygroundWebview,
  showPlaygroundWebview,
} from "./webview.ts";

const KEEPER_BOUNDS = { x: -64, y: -64, width: 64, height: 64 };
const domBaselines = new Map<string, string>();

export function usePlaygroundRuntime() {
  usePlaygroundFenceWatcher();
  usePlaygroundWebviewKeeper();
  usePlaygroundUpdatePolling();
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
      // OS split/playground owns a window-scoped child. Do not remount the
      // main-window playground-{sid} label underneath — hide leftovers.
      if (osHosted.has(session.sid)) {
        void hidePlaygroundWebview(session.sid);
        continue;
      }
      void showPlaygroundWebview({
        sid: session.sid,
        url: session.url,
        bounds: KEEPER_BOUNDS,
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
