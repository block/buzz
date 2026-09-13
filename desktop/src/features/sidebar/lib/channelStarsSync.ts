import { KIND_CHANNEL_STARS } from "@/shared/constants/kinds";
import {
  clearChannelStarsOutbox,
  isStarsStoreSubsumedBy,
  mergeStores,
  parseStarPayload,
  writeChannelStarsOutbox,
  type ChannelStarStore,
} from "./channelStarsStorage";
import {
  MergeLaneSyncManager,
  type RemoteMergeBlob,
} from "./mergeLaneSyncManager";

export type RemoteStars = RemoteMergeBlob<ChannelStarStore>;

function starsStoresEqual(a: ChannelStarStore, b: ChannelStarStore): boolean {
  const aKeys = Object.keys(a.channels);
  const bKeys = Object.keys(b.channels);
  if (aKeys.length !== bKeys.length) return false;
  for (const key of aKeys) {
    const ae = a.channels[key];
    const be = b.channels[key];
    if (
      !ae ||
      !be ||
      ae.starred !== be.starred ||
      ae.updatedAt !== be.updatedAt ||
      ae.rev !== be.rev
    )
      return false;
  }
  return true;
}

export class ChannelStarSyncManager extends MergeLaneSyncManager<ChannelStarStore> {
  constructor(pubkey: string, relayUrl: string) {
    super(pubkey, relayUrl, {
      kind: KIND_CHANNEL_STARS,
      dTag: "channel-stars",
      logPrefix: "channelStarsSync",
      publishTimeoutMsg: "Timed out publishing channel stars.",
      publishErrorMsg: "Failed to publish channel stars.",
      parse: parseStarPayload,
      serializePayload: (store) => ({ version: 1, channels: store.channels }),
      mergeWithRemote: (local, remote, preservedKey) =>
        mergeStores(local, remote, preservedKey),
      isSubsumedBy: isStarsStoreSubsumedBy,
      storesEqual: starsStoresEqual,
      observe: (highWater, store) => {
        for (const [id, entry] of Object.entries(store.channels)) {
          const cur = highWater.get(id) ?? { rev: 0, updatedAt: 0 };
          highWater.set(id, {
            rev: Math.max(cur.rev, entry.rev),
            updatedAt: Math.max(cur.updatedAt, entry.updatedAt),
          });
        }
      },
      writeOutbox: writeChannelStarsOutbox,
      clearOutbox: clearChannelStarsOutbox,
      isLocalNonEmpty: (s) => Object.keys(s.channels).length > 0,
    });
  }

  /** Publish a star store, debounced to 2s. */
  publishStars(store: ChannelStarStore, preservedKey?: string): void {
    this.publish(store, preservedKey);
  }

  /**
   * Re-drive the current pending star edit without resetting the preserved key
   * — used by the reconnect handler (Kalvin P3).
   */
  retryReconnectStarsPublish(): void {
    this.retryReconnectPublish();
  }

  /** Fetch the current remote head for this pubkey's stars blob. */
  fetchRemoteStars() {
    return this.fetchRemoteBlob();
  }

  /** Cancel any pending star publish (debounce or retry timer). */
  cancelPendingStarPublish(): void {
    this.cancelPendingPublish();
  }

  /** The currently pending store, or null if nothing is queued. */
  getPendingStarStore(): ChannelStarStore | null {
    return this.getPendingStore();
  }

  /** Subscribe to live relay events for this pubkey's stars blob. */
  subscribeToStars(onUpdate: (remote: RemoteStars) => void) {
    return this.subscribeLive(onUpdate);
  }
}
