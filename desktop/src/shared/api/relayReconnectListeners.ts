/** Notify mounted repair owners after an established connection is replaced. */
export function notifyRelayReconnectListeners(
  notify: boolean,
  listeners: Iterable<() => void>,
) {
  if (!notify) return;
  for (const listener of listeners) {
    try {
      listener();
    } catch (error) {
      console.error("Failed to handle relay reconnect", error);
    }
  }
}
