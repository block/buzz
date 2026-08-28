import type { RelayEvent } from "@/shared/api/types";

type Dispose = () => void | Promise<void>;

type SubscriptionOwner = {
  onEvent: (event: RelayEvent) => void;
  refresh: () => Promise<void>;
};

type RegistryDependencies = {
  onError: (message: string, channelId: string, error: unknown) => void;
  subscribe: (
    channelId: string,
    onEvent: (event: RelayEvent) => void,
  ) => Promise<Dispose>;
  subscribeToReconnects: (onReconnect: () => void) => Dispose;
};

type SubscriptionEntry = {
  disposeReconnect: Dispose;
  disposeSubscription?: Dispose;
  owners: Map<symbol, SubscriptionOwner>;
};

export function createChannelLiveSubscriptionRegistry({
  onError,
  subscribe,
  subscribeToReconnects,
}: RegistryDependencies) {
  const entries = new Map<string, SubscriptionEntry>();
  const currentOwner = (entry: SubscriptionEntry) =>
    Array.from(entry.owners.values()).at(-1);

  const refresh = (
    entry: SubscriptionEntry,
    channelId: string,
    outcome: string,
  ) => {
    const owner = currentOwner(entry);
    if (!owner) return;
    void owner.refresh().catch((error) => {
      if (entries.get(channelId) === entry) {
        onError(
          `Failed to refresh channel window after ${outcome}`,
          channelId,
          error,
        );
      }
    });
  };

  return {
    acquire(channelId: string, owner: SubscriptionOwner) {
      const token = Symbol(channelId);
      let entry = entries.get(channelId);
      if (!entry) {
        const owners = new Map<symbol, SubscriptionOwner>([[token, owner]]);
        entry = {
          disposeReconnect: subscribeToReconnects(() => {
            const current = entries.get(channelId);
            if (current) refresh(current, channelId, "reconnecting");
          }),
          owners,
        };
        entries.set(channelId, entry);
        const startedEntry = entry;
        void subscribe(channelId, (event) => {
          currentOwner(startedEntry)?.onEvent(event);
        }).then(
          (dispose) => {
            if (entries.get(channelId) !== startedEntry) {
              void dispose();
              return;
            }
            startedEntry.disposeSubscription = dispose;
            refresh(startedEntry, channelId, "subscribing");
          },
          (error) => {
            if (entries.get(channelId) !== startedEntry) return;
            onError("Failed to subscribe to channel", channelId, error);
            refresh(startedEntry, channelId, "subscription failure");
          },
        );
      } else {
        entry.owners.set(token, owner);
      }

      let released = false;
      return () => {
        if (released) return;
        released = true;
        const current = entries.get(channelId);
        if (!current) return;
        current.owners.delete(token);
        if (current.owners.size > 0) return;
        entries.delete(channelId);
        void current.disposeReconnect();
        if (current.disposeSubscription) void current.disposeSubscription();
      };
    },
  };
}
