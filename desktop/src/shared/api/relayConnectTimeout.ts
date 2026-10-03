/**
 * Race a tauri-plugin-websocket `connect` invoke against a hard timeout.
 *
 * The plugin holds a global connection-manager mutex while awaiting `send()`.
 * A stuck `send()` from a previous dead connection can starve any subsequent
 * `connect()` registration indefinitely — the exact stuck state in issue #3975
 * where the Desktop client shows "can't connect to relay" and manual Reconnect
 * appears to do nothing because `connectPromise` never settles. Racing the
 * invoke guarantees the future settles: on timeout we reject so the normal
 * connection-failure path runs (backoff → retry), and manual Reconnect always
 * starts a fresh attempt instead of joining a stuck one.
 *
 * Extracted as a pure function (no class state) so the timeout path can be
 * unit-tested without the Tauri runtime, following the same pattern as
 * `relayStallWatchdog.ts`.
 */
export type ConnectInvokeFn = (
  command: string,
  args: Record<string, unknown>,
) => Promise<number>;

export function connectWebSocketWithTimeout(
  invokeFunc: ConnectInvokeFn,
  relayUrl: string,
  onMessageChannel: unknown,
  timeoutMs: number,
  setTimeoutFn: (fn: () => void, ms: number) => number = window.setTimeout,
  clearTimeoutFn: (id: number) => void = window.clearTimeout,
): Promise<number> {
  return new Promise<number>((resolve, reject) => {
    const timeout = setTimeoutFn(() => {
      reject(
        new Error(
          `Relay websocket connect invoke timed out after ${timeoutMs}ms.`,
        ),
      );
    }, timeoutMs);

    invokeFunc("plugin:websocket|connect", {
      url: relayUrl,
      onMessage: onMessageChannel,
      config: {},
    })
      .then((wsId) => {
        clearTimeoutFn(timeout);
        resolve(wsId);
      })
      .catch((error) => {
        clearTimeoutFn(timeout);
        reject(error);
      });
  });
}
