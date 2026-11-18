import type { QueryClient } from "@tanstack/react-query";
import {
  loadChannelHeadCache,
  type ChannelHeadScope,
} from "@/shared/api/tauriChannelHeadCache";
import { channelMessagesKey, channelWindowKey } from "./messageQueryKeys";
import { parseChannelWindowResponse } from "./channelWindowResponse";
import {
  type ChannelWindowStore,
  emptyChannelWindowStore,
  replaceNewestChannelWindow,
} from "./channelWindowStore";
import { reconcileChannelWindowMessages } from "./channelWindowReconciliation";
import type { RelayEvent } from "@/shared/api/types";
const hydrations = new WeakMap<QueryClient, Promise<void>>();
const hydratedChannels = new WeakMap<QueryClient, Set<string>>();
const persistedHydratedChannels = new WeakMap<QueryClient, Set<string>>();
const cacheScopes = new WeakMap<QueryClient, ChannelHeadScope>();
export function isChannelHeadCacheEnabled(): boolean {
  if (typeof window === "undefined") return false;
  if (import.meta.env?.VITE_BUZZ_CHANNEL_HEAD_CACHE === "off") return false;
  return window.localStorage.getItem("buzz-channel-head-cache") !== "off";
}
export function channelHeadCacheScope(
  queryClient: QueryClient,
): ChannelHeadScope | null {
  return cacheScopes.get(queryClient) ?? null;
}
/**
 * Resolves once persisted heads have been seeded into this client (or
 * immediately when no hydration was started). The channel query awaits this so
 * it never races the cache load with a cold relay fetch, while the rest of the
 * app mounts without waiting on the optional paint cache.
 */
export function channelHeadHydration(queryClient: QueryClient): Promise<void> {
  return hydrations.get(queryClient) ?? Promise.resolve();
}
export function consumeHydratedChannel(
  queryClient: QueryClient,
  channelId: string,
): boolean {
  const channels = hydratedChannels.get(queryClient);
  if (!channels?.delete(channelId)) return false;
  if (channels.size === 0) hydratedChannels.delete(queryClient);
  return true;
}
export function hasPersistedHydratedChannel(
  queryClient: QueryClient,
  channelId: string,
): boolean {
  return persistedHydratedChannels.get(queryClient)?.has(channelId) ?? false;
}
export function hydrateChannelHeads(
  queryClient: QueryClient,
  scope: ChannelHeadScope,
): Promise<void> {
  const hydration = seedChannelHeads(queryClient, scope).catch((error) => {
    console.warn("Failed to hydrate persisted channel heads", error);
  });
  hydrations.set(queryClient, hydration);
  return hydration;
}
async function seedChannelHeads(
  queryClient: QueryClient,
  scope: ChannelHeadScope,
): Promise<void> {
  if (!isChannelHeadCacheEnabled()) return;
  cacheScopes.set(queryClient, scope);
  const entries = await loadChannelHeadCache(scope, 12);
  const hydrated = new Set<string>();
  for (const entry of entries) {
    try {
      const page = parseChannelWindowResponse(
        entry.events,
        entry.channelId,
        null,
      );
      // A bounds-only head has nothing to paint; let it take the cold path so
      // the skeleton holds until the relay answers instead of flashing the
      // empty-channel intro over rows that are still revalidating.
      if (page.rows.length === 0) continue;
      const windowKey = channelWindowKey(entry.channelId);
      const messagesKey = channelMessagesKey(entry.channelId);
      // Merge, don't replace: the app mounts while this load is in flight, so
      // the live subscription may already have overlaid events on this store.
      const window = replaceNewestChannelWindow(
        queryClient.getQueryData<ChannelWindowStore>(windowKey) ??
          emptyChannelWindowStore(),
        page,
      );
      const messages = reconcileChannelWindowMessages(
        window,
        queryClient.getQueryData<RelayEvent[]>(messagesKey) ?? [],
      );
      queryClient.setQueryData(windowKey, window, { updatedAt: 0 });
      queryClient.setQueryData(messagesKey, messages, { updatedAt: 0 });
      hydrated.add(entry.channelId);
    } catch (error) {
      console.warn(
        "Ignoring invalid persisted channel head",
        entry.channelId,
        error,
      );
    }
  }
  if (hydrated.size > 0) {
    hydratedChannels.set(queryClient, hydrated);
    persistedHydratedChannels.set(queryClient, new Set(hydrated));
  }
}
