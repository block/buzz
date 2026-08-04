import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";
import {
  DEFAULT_MANUAL_ORDER_STORE,
  parseChannelManualOrderPayload,
  type ChannelManualOrderStore,
} from "./channelManualOrder";

const STORAGE_KEY_PREFIX = "buzz-channel-manual-order.v1";

export function channelManualOrderStorageKey(
  pubkey: string,
  relayUrl?: string,
): string {
  if (!relayUrl) return `${STORAGE_KEY_PREFIX}:${pubkey}`;
  return `${STORAGE_KEY_PREFIX}:${pubkey}:${encodeURIComponent(
    normalizeRelayUrl(relayUrl),
  )}`;
}

export function readChannelManualOrderStore(
  pubkey: string,
  relayUrl?: string,
): ChannelManualOrderStore {
  try {
    const raw = window.localStorage.getItem(
      channelManualOrderStorageKey(pubkey, relayUrl),
    );
    if (!raw) return DEFAULT_MANUAL_ORDER_STORE;
    return (
      parseChannelManualOrderPayload(JSON.parse(raw)) ??
      DEFAULT_MANUAL_ORDER_STORE
    );
  } catch {
    return DEFAULT_MANUAL_ORDER_STORE;
  }
}

export function writeChannelManualOrderStore(
  pubkey: string,
  store: ChannelManualOrderStore,
  relayUrl?: string,
): boolean {
  try {
    window.localStorage.setItem(
      channelManualOrderStorageKey(pubkey, relayUrl),
      JSON.stringify(store),
    );
    return true;
  } catch {
    return false;
  }
}
