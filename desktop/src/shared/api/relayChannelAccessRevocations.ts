import type { RelaySubscription } from "./relayClientShared";

/** Session-owned hints; CLOSED never establishes archive or membership state. */
export class RelayChannelAccessRevocations {
  private listeners = new Set<{ listener: () => void; channel?: string }>();

  /** Optionally restrict hints to one channel without changing unscoped consumers. */
  subscribe(listener: () => void, channel?: string) {
    const entry = { listener, channel };
    this.listeners.add(entry);
    return () => {
      this.listeners.delete(entry);
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
      for (const entry of this.listeners) {
        if (
          entry.channel === undefined ||
          subscription.filter["#h"]?.includes(entry.channel)
        ) {
          entry.listener();
        }
      }
    }
  }
}
