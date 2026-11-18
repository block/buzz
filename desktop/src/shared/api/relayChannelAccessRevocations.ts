import type { RelaySubscription } from "./relayClientShared";

/** Session-owned hints; CLOSED never establishes archive or membership state. */
export class RelayChannelAccessRevocations {
  private listeners = new Set<() => void>();

  subscribe(listener: () => void) {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  clear() {
    this.listeners.clear();
  }

  /** Call after terminal cleanup, using the subscription captured before removal. */
  notify(subscription: RelaySubscription | undefined, reason: unknown) {
    if (
      subscription?.mode === "live" &&
      (subscription.filter["#h"]?.length ?? 0) > 0 &&
      reason === "restricted: channel access revoked"
    ) {
      for (const listener of this.listeners) listener();
    }
  }
}
