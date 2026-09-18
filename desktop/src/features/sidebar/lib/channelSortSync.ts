import { KIND_CHANNEL_SORT } from "@/shared/constants/kinds";
import {
  clearChannelSortOutbox,
  parseChannelSortPayload,
  writeChannelSortOutbox,
  type ChannelSortStore,
} from "./channelSortPreference";
import { WholeBlobSyncManager, type RemoteBlob } from "./wholeBlobSyncManager";

export type RemoteSortPrefs = RemoteBlob<ChannelSortStore>;

function sortStoresEqual(a: ChannelSortStore, b: ChannelSortStore): boolean {
  const aKeys = Object.keys(a.groups);
  const bKeys = Object.keys(b.groups);
  if (aKeys.length !== bKeys.length) return false;
  for (const key of aKeys) {
    if (a.groups[key] !== b.groups[key]) return false;
  }
  return true;
}

export class ChannelSortSyncManager extends WholeBlobSyncManager<ChannelSortStore> {
  constructor(pubkey: string, relayUrl: string) {
    super(pubkey, relayUrl, {
      kind: KIND_CHANNEL_SORT,
      dTag: "channel-sort",
      logPrefix: "channelSortSync",
      parse: parseChannelSortPayload,
      serializePayload: (store) => ({
        version: 1,
        groups: store.groups,
      }),
      writeOutbox: writeChannelSortOutbox,
      clearOutbox: clearChannelSortOutbox,
      storesEqual: sortStoresEqual,
      isLocalNonEmpty: (s) => Object.keys(s.groups).length > 0,
    });
  }

  /** Publish a sort prefs store, debounced to 2s. Returns whether the intent is durably held. */
  publishSortPrefs(
    store: ChannelSortStore,
    isRestoredReplay = false,
    restoredQueuedAt?: number,
  ): boolean {
    return this.publish(store, isRestoredReplay, restoredQueuedAt);
  }

  /** Fetch the current remote head for this pubkey's sort-prefs blob. */
  fetchRemoteSortPrefs() {
    return this.fetchRemoteBlob();
  }

  /** Subscribe to live relay events for this pubkey's sort-prefs blob. */
  subscribeToSortPrefs(onUpdate: (remote: RemoteSortPrefs) => void) {
    return this.subscribeLive(onUpdate);
  }
}
