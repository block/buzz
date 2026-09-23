type SubscriptionFilter = {
  "#h"?: readonly string[];
  "#p"?: readonly string[];
  kinds?: readonly number[];
};

/** Preserve raw REQ correlation alongside legacy mock delivery projections. */
export function createMockSubscription(filters: readonly SubscriptionFilter[]) {
  const channelIds = [
    ...new Set(filters.flatMap((filter) => filter["#h"] ?? [])),
  ];
  const kinds = [...new Set(filters.flatMap((filter) => filter.kinds ?? []))];
  return {
    filters,
    channelIds: channelIds.length ? channelIds : ["*"],
    kinds: kinds.length ? kinds : null,
    ownerPubkeys: [...new Set(filters.flatMap((filter) => filter["#p"] ?? []))],
  };
}

/** Test readiness must distinguish a channel consumer from unrelated global REQs. */
export function hasMockSubscription(
  subscriptions: Iterable<ReturnType<typeof createMockSubscription>>,
  channelId: string,
  kind?: number,
  exactChannel = false,
): boolean {
  for (const subscription of subscriptions) {
    if (exactChannel) {
      if (
        subscription.filters.some(
          (filter) =>
            filter["#h"]?.includes(channelId) &&
            (kind === undefined ||
              !filter.kinds?.length ||
              filter.kinds.includes(kind)),
        )
      )
        return true;
    } else if (
      (subscription.channelIds.includes(channelId) ||
        subscription.channelIds.includes("*")) &&
      (kind === undefined ||
        !subscription.kinds ||
        subscription.kinds.includes(kind))
    )
      return true;
  }
  return false;
}
