import type { UnlistenFn } from "@tauri-apps/api/event";

/**
 * `listen()` resolves with its event id as soon as the backend command
 * returns, but the backend publishes that id into the webview through a
 * separate `eval`. An unlisten issued before that eval lands throws from
 * Tauri's injected `unregisterListener` (`listeners[eventId]` is still
 * undefined) *before* it invokes `plugin:event|unlisten`, so the backend
 * listener survives and keeps calling the handler. Retrying past the gap is
 * what actually removes it; swallowing the error would leak the listener.
 */
const RETRY_DELAYS_MS = [0, 16, 64, 256] as const;

export type UnlistenRetryHost = {
  setTimeout: (handler: () => void, ms: number) => void;
};

const defaultHost: UnlistenRetryHost = {
  setTimeout: (handler, ms) => {
    window.setTimeout(handler, ms);
  },
};

async function attemptUnlisten(
  unlisten: UnlistenFn,
  attempt: number,
  host: UnlistenRetryHost,
): Promise<void> {
  try {
    await unlisten();
  } catch (error) {
    const delayMs = RETRY_DELAYS_MS[attempt];
    if (delayMs === undefined) {
      console.debug("[tauri] gave up removing an event listener:", error);
      return;
    }
    await new Promise<void>((resolve) => host.setTimeout(resolve, delayMs));
    await attemptUnlisten(unlisten, attempt + 1, host);
  }
}

/**
 * Removes a Tauri event listener without letting the teardown race surface as
 * an unhandled rejection. Safe to call from an effect cleanup and safe to call
 * more than once for the same listener.
 */
export function safeUnlisten(
  unlisten: UnlistenFn | null | undefined,
  host: UnlistenRetryHost = defaultHost,
): void {
  if (!unlisten) return;
  void attemptUnlisten(unlisten, 0, host);
}
