import { invokeTauri } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
export type ChannelHeadScope = { pubkey: string; relayUrl: string };
export type ChannelHeadEntry = {
  channelId: string;
  events: RelayEvent[];
  savedAt: number;
  lastVisitedAt: number;
};
export function loadChannelHeadCache(
  scope: ChannelHeadScope,
  limit = 12,
): Promise<ChannelHeadEntry[]> {
  return invokeTauri("channel_head_cache_load", { scope, limit });
}
export function storeChannelHeadCache(
  scope: ChannelHeadScope,
  channelId: string,
  events: RelayEvent[],
): Promise<void> {
  return invokeTauri("channel_head_cache_store", { scope, channelId, events });
}
export function clearChannelHeadCache(scope: ChannelHeadScope): Promise<void> {
  return invokeTauri("channel_head_cache_clear", { scope });
}
