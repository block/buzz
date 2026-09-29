import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import * as React from "react";

/**
 * Unexpected audio-owner/pod disconnects are recoverable: keep the huddle,
 * mic, and voice pipelines live while Rust reconnects only the audio WS.
 * `tokenRef` makes an intentional leave/start supersede this loop, and the
 * in-flight guard collapses duplicate disconnect events from failed dials.
 */
export function useHuddleAudioReconnect(options: {
  ownsAudioSession: boolean;
  tokenRef: React.MutableRefObject<number>;
  leaveHuddleRef: React.MutableRefObject<() => Promise<boolean>>;
}): void {
  const { ownsAudioSession, tokenRef, leaveHuddleRef } = options;
  const audioReconnectInFlightRef = React.useRef(false);

  React.useEffect(() => {
    if (!ownsAudioSession) return;

    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen("huddle-audio-disconnected", () => {
      if (cancelled || audioReconnectInFlightRef.current) return;
      audioReconnectInFlightRef.current = true;
      const reconnectToken = tokenRef.current;

      void (async () => {
        // Keep a long enough tail for Kubernetes Service endpoint removal after
        // a draining pod flips readiness. Early retries make remote-owner
        // handoff fast; the two 2s attempts prevent a client connected to the
        // draining pod itself from exhausting before kube-proxy converges.
        const delaysMs = [0, 100, 250, 500, 1_000, 2_000, 2_000];
        for (const delayMs of delaysMs) {
          if (cancelled || tokenRef.current !== reconnectToken) return;
          if (delayMs > 0) {
            await new Promise((resolve) => window.setTimeout(resolve, delayMs));
          }
          if (cancelled || tokenRef.current !== reconnectToken) return;
          try {
            await invoke("reconnect_huddle_audio");
            // Success installs a live replacement pipeline. If it later fails,
            // its Tauri event arrives after this loop releases the in-flight
            // guard and starts a fresh bounded recovery cycle. Repeating those
            // cycles is intentional while the relay remains connectable.
            return;
          } catch {
            // A draining pod may still receive the first retry before Service
            // endpoints converge. Keep the bounded backoff client-local.
          }
        }

        if (!cancelled && tokenRef.current === reconnectToken) {
          await leaveHuddleRef.current();
        }
      })().finally(() => {
        audioReconnectInFlightRef.current = false;
      });
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [leaveHuddleRef, ownsAudioSession, tokenRef]);
}
