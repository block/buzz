type Subscription = {
  channelIds: readonly string[];
  kinds: readonly number[] | null;
};

/** Test readiness must distinguish a channel consumer from unrelated global REQs. */
export function hasMockSubscription(
  subscriptions: Iterable<Subscription>,
  channelId: string,
  kind?: number,
  exactChannel = false,
): boolean {
  for (const subscription of subscriptions) {
    if (
      (subscription.channelIds.includes(channelId) ||
        (!exactChannel && subscription.channelIds.includes("*"))) &&
      (kind === undefined ||
        !subscription.kinds ||
        subscription.kinds.includes(kind))
    )
      return true;
  }
  return false;
}
