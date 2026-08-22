export const RECONNECT_BASE_DELAY_MS = 1_000;
export const RECONNECT_MAX_DELAY_MS = 30_000;
export const EVENT_BATCH_MS = 16;

/**
 * Op-level timeouts tolerate degraded networks where TLS handshakes and DNS
 * resolution can take several seconds.
 */
export const AUTH_TIMEOUT_MS = 25_000;
export const HISTORY_TIMEOUT_MS = 25_000;
export const PUBLISH_TIMEOUT_MS = 25_000;

/**
 * Hard timeout for the `plugin:websocket|connect` invoke itself.
 *
 * tauri-plugin-websocket holds a global connection-manager mutex while awaiting
 * `send()`; a stuck `send()` from a previous dead connection can therefore starve
 * any subsequent `connect()` registration indefinitely (see issue #3975). Unlike
 * `AUTH_TIMEOUT_MS` (which guards the post-handshake auth round-trip) this guards
 * the plugin's own registration path, which has no other timeout. On fire the
 * retry wrapper treats it like a normal connection failure and backs off, so a
 * manual Reconnect click always re-enters a fresh attempt instead of joining a
 * stuck future.
 */
export const RECONNECT_INVOKE_TIMEOUT_MS = 30_000;

/**
 * A stability-gated reset prevents reconnect flapping from erasing backoff.
 */
export const BACKOFF_RESET_STABLE_MS = 60_000;

/** Passive liveness thresholds for the relay heartbeat stream. */
export const STALL_CHECK_INTERVAL_MS = 10_000;
export const STALL_IDLE_TIMEOUT_MS = 60_000;
