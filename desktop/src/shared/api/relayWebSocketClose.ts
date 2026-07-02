import { invoke } from "@tauri-apps/api/core";

/**
 * tauri-plugin-websocket 2.4.2 registers only `connect` and `send` — there is
 * no `disconnect` command, so invoking one rejects and the socket leaks. Close
 * the way the plugin's own JS API does: send a Close frame; the plugin's read
 * loop drops the connection when the close handshake completes.
 */
export function closeWebSocket(
  id: number,
  reason: string,
  invokeFn: typeof invoke = invoke,
): Promise<void> {
  return invokeFn("plugin:websocket|send", {
    id,
    message: {
      type: "Close",
      data: { code: 1000, reason },
    },
  }).then(
    () => undefined,
    () => {
      // Socket already gone (or send path wedged) — nothing left to release.
    },
  );
}
