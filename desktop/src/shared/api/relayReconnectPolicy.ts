/**
 * Pure helpers for the relay reconnect policy.
 *
 * Extracted from `RelayClient` so the decision rules — when to schedule a
 * reconnect, when to refuse to connect — live in one legible place that
 * unit tests can reach without booting the WS layer.
 *
 * The rules:
 *
 *  - **Terminal sessions never reconnect.** When the relay has explicitly
 *    rejected us (today: kind:22242 AUTH OK=false) the session is dead
 *    until the user re-engages (workspace switch or explicit preconnect).
 *    This guards the reconnect-timer catch handler — and the retry wrappers
 *    in `publishEvent` / `sendRawWithReconnectRetry` — from racing the
 *    `disconnected` state back to `reconnecting`.
 *
 *  - **No-op when a reconnect is already scheduled or in progress.**
 *    A pending timer or a live `wsId` means we have nothing to do.
 *
 *  - **No reconnect needed when nothing wants the socket.** No live
 *    subscription, no `keepAliveRequested` from `preconnect()` → don't
 *    keep an idle socket.
 */
export type RelayReconnectInputs = {
  terminal: boolean;
  hasPendingReconnect: boolean;
  hasLiveSocket: boolean;
  keepAliveRequested: boolean;
  hasLiveSubscriptions: boolean;
};

export function shouldScheduleReconnect(inputs: RelayReconnectInputs): boolean {
  if (inputs.terminal) return false;
  if (inputs.hasPendingReconnect) return false;
  if (inputs.hasLiveSocket) return false;
  if (!inputs.keepAliveRequested && !inputs.hasLiveSubscriptions) return false;
  return true;
}

/** Whether `ensureConnected()` should refuse with a terminal error. */
export function shouldRefuseConnect(inputs: { terminal: boolean }): boolean {
  return inputs.terminal;
}
