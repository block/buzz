import type { RelaySubscription } from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";

export function deliverBufferedSubscriptionEvents(
  buffer: Array<{ subId: string; event: RelayEvent }>,
  subscriptions: Map<string, RelaySubscription>,
) {
  // Re-lookup: subscriptions removed during the batch window are skipped.
  for (const { subId, event } of buffer) {
    const subscription = subscriptions.get(subId);
    if (subscription?.mode === "live") {
      subscription.onEvent(event);
    }
  }
}
