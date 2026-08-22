import type { QueryClient } from "@tanstack/react-query";
import {
  loadChannelHeadCache,
  type ChannelHeadScope,
} from "@/shared/api/tauriChannelHeadCache";
import { channelMessagesKey, channelWindowKey } from "./messageQueryKeys";
import { parseChannelWindowResponse } from "./channelWindowResponse";
import {
  emptyChannelWindowStore,
  replaceNewestChannelWindow,
} from "./channelWindowStore";
import { reconcileChannelWindowMessages } from "./channelWindowReconciliation";
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
export async function hydrateChannelHeads(
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
      const window = replaceNewestChannelWindow(
        emptyChannelWindowStore(),
        page,
      );
      const messages = reconcileChannelWindowMessages(window, []);
      queryClient.setQueryData(channelWindowKey(entry.channelId), window, {
        updatedAt: 0,
      });
      queryClient.setQueryData(channelMessagesKey(entry.channelId), messages, {
        updatedAt: 0,
      });
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
