import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_HUDDLE_SCREEN } from "@/shared/constants/kinds";
import { startScreenPublisher } from "./lib/screenPublisher";
import {
  ScreenShareReceiver,
  type ScreenShare,
  type SharedScreen,
} from "./lib/screenShare";

type ScreenState = {
  phase: string;
  ephemeral_channel_id: string | null;
  screen_share?: ScreenShare | null;
  screen_share_error?: string | null;
};

type ScreenContextValue = {
  channelId: string | null;
  localShare: ScreenShare | null;
  error: string | null;
  screens: SharedScreen[];
};

const ScreenContext = React.createContext<ScreenContextValue>({
  channelId: null,
  localShare: null,
  error: null,
  screens: [],
});

/** Screen controls and live frames scoped to this webview's current huddle. */
export function useHuddleScreen() {
  return React.useContext(ScreenContext);
}

/** The main window publishes; companion windows only mirror and view. */
export function HuddleScreenProvider({
  channelId: ownerChannelId,
  ownsCapture,
  children,
}: {
  channelId: string | null;
  ownsCapture: boolean;
  children: React.ReactNode;
}) {
  const [backend, setBackend] = React.useState<ScreenState | null>(null);
  const [screens, setScreens] = React.useState<SharedScreen[]>([]);
  const [receiveError, setReceiveError] = React.useState<string | null>(null);
  const backendRef = React.useRef(backend);
  backendRef.current = backend;
  const channelId = ownsCapture
    ? ownerChannelId
    : (backend?.ephemeral_channel_id ?? null);
  const active =
    backend?.ephemeral_channel_id === channelId &&
    (backend.phase === "active" || backend.phase === "connected");
  const localShare = active ? (backend.screen_share ?? null) : null;
  const localShareRef = React.useRef(localShare);
  localShareRef.current = localShare;

  React.useEffect(() => {
    let disposed = false;
    let revision = 0;
    let unlisten: (() => void) | undefined;
    // Install the listener before fetching; reject a stale initial snapshot.
    void listen<ScreenState>("huddle-state-changed", ({ payload }) => {
      if (disposed) return;
      revision++;
      setBackend(payload);
    })
      .then(async (cleanup) => {
        if (disposed) {
          cleanup();
          return;
        }
        unlisten = cleanup;
        const fetchedRevision = revision;
        try {
          const state = await invoke<ScreenState>("get_huddle_state");
          if (!disposed && revision === fetchedRevision) setBackend(state);
        } catch (error) {
          if (!disposed)
            setReceiveError(`Screen sharing unavailable: ${String(error)}`);
        }
      })
      .catch((error) => {
        if (!disposed)
          setReceiveError(`Screen sharing unavailable: ${String(error)}`);
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  React.useEffect(() => {
    setScreens([]);
    setReceiveError(null);
    if (!channelId || !active) return;
    let disposed = false;
    const receiver = new ScreenShareReceiver(channelId);
    const abort = new AbortController();
    const sync = () => {
      if (!disposed) setScreens(receiver.snapshot());
    };
    const clear = () => {
      receiver.clear();
      sync();
    };
    void relayClient
      .subscribeLive(
        { kinds: [KIND_HUDDLE_SCREEN], "#h": [channelId], limit: 0 },
        (event) => {
          if (!disposed && receiver.receive(event)) {
            setReceiveError(null);
            sync();
          }
        },
        undefined,
        undefined,
        abort.signal,
      )
      .catch((error) => {
        if (!disposed)
          setReceiveError(`Could not receive shared screens: ${String(error)}`);
      });
    const unsubscribeConnection = relayClient.subscribeToConnectionState(
      (state) => {
        if (state !== "connected") clear();
      },
    );
    const unsubscribeAccess = relayClient.subscribeToChannelAccessRevocations(
      () => {
        clear();
        if (ownsCapture && localShareRef.current) {
          void invoke("stop_huddle_screen_share", {
            sessionId: localShareRef.current.session_id,
            error: "Screen sharing stopped because channel access changed",
          }).catch((error) => {
            if (!disposed) setReceiveError(String(error));
          });
        }
      },
      channelId,
    );
    const timer = window.setInterval(() => {
      if (receiver.expire()) sync();
    }, 500);
    return () => {
      disposed = true;
      abort.abort();
      window.clearInterval(timer);
      unsubscribeConnection();
      unsubscribeAccess();
      receiver.clear();
    };
  }, [active, channelId, ownsCapture]);

  const sessionId = localShare?.session_id;
  React.useEffect(() => {
    if (!ownsCapture || !channelId || !sessionId) return;
    const share = localShareRef.current;
    if (!share) return;
    let current = true;
    const isCurrent = () =>
      current && localShareRef.current?.session_id === sessionId;
    const stop = startScreenPublisher({
      capture: async (refreshKeyframe) => {
        const json = await invoke<string | null>(
          "read_huddle_screen_video_chunk",
          { sessionId, refreshKeyframe },
        );
        if (!json && isCurrent()) {
          const state = await invoke<ScreenState>("get_huddle_state");
          if (isCurrent()) setBackend(state);
        }
        return json ? (JSON.parse(json) as RelayEvent) : null;
      },
      publish: (event, alive) =>
        relayClient.publishEvent(
          event,
          "Screen sharing timed out",
          "Could not publish shared screen",
          () => alive() && isCurrent(),
        ),
      fail: async (error) => {
        if (!isCurrent()) return;
        try {
          await invoke("stop_huddle_screen_share", {
            sessionId,
            error: `Screen sharing stopped: ${String(error)}`,
          });
        } catch (stopError) {
          setReceiveError(
            `Could not stop screen sharing: ${String(stopError)}`,
          );
        }
      },
    });
    return () => {
      current = false;
      stop();
      // Stops are ephemeral too. A failed terminal send is recovered by the
      // viewer's expiry, never by replaying screen pixels after reconnect.
      if (backendRef.current?.ephemeral_channel_id !== channelId) return;
      void signRelayEvent({
        kind: KIND_HUDDLE_SCREEN,
        tags: [["h", channelId]],
        content: JSON.stringify({ type: "stopped", share }),
      })
        .then((event) =>
          relayClient.publishEvent(
            event,
            "Screen stop timed out",
            "Could not notify screen viewers",
            () => backendRef.current?.ephemeral_channel_id === channelId,
          ),
        )
        .catch(() => {
          // Remote views expire within five seconds even when the relay is down.
        });
    };
  }, [channelId, ownsCapture, sessionId]);

  const value = React.useMemo(
    () => ({
      channelId: active ? channelId : null,
      localShare,
      error: backend?.screen_share_error ?? receiveError,
      screens: active ? screens : [],
    }),
    [
      active,
      backend?.screen_share_error,
      channelId,
      localShare,
      receiveError,
      screens,
    ],
  );
  return (
    <ScreenContext.Provider value={value}>{children}</ScreenContext.Provider>
  );
}
