import { KIND_CHANNEL_MUTES } from "@/shared/constants/kinds";
import {
  clearChannelMutesOutbox,
  isMutesStoreSubsumedBy,
  mergeStores,
  parseMutePayload,
  writeChannelMutesOutbox,
  type ChannelMuteStore,
} from "./channelMutesStorage";
import {
  MergeLaneSyncManager,
  type RemoteMergeBlob,
} from "./mergeLaneSyncManager";

export type RemoteMutes = RemoteMergeBlob<ChannelMuteStore>;

function mutesStoresEqual(a: ChannelMuteStore, b: ChannelMuteStore): boolean {
  const aKeys = Object.keys(a.channels);
  const bKeys = Object.keys(b.channels);
  if (aKeys.length !== bKeys.length) return false;
  for (const key of aKeys) {
    const ae = a.channels[key];
    const be = b.channels[key];
    if (
      !ae ||
      !be ||
      ae.muted !== be.muted ||
      ae.updatedAt !== be.updatedAt ||
      ae.rev !== be.rev
    )
      return false;
  }
  return true;
}

export class ChannelMuteSyncManager extends MergeLaneSyncManager<ChannelMuteStore> {
  constructor(pubkey: string, relayUrl: string) {
    super(pubkey, relayUrl, {
      kind: KIND_CHANNEL_MUTES,
      dTag: "channel-mutes",
      logPrefix: "channelMutesSync",
      publishTimeoutMsg: "Timed out publishing channel mutes.",
      publishErrorMsg: "Failed to publish channel mutes.",
      parse: parseMutePayload,
      serializePayload: (store) => ({ version: 1, channels: store.channels }),
      mergeWithRemote: (local, remote, preservedKey) =>
        mergeStores(local, remote, preservedKey),
      isSubsumedBy: isMutesStoreSubsumedBy,
      storesEqual: mutesStoresEqual,
      observe: (highWater, store) => {
        for (const [id, entry] of Object.entries(store.channels)) {
          const cur = highWater.get(id) ?? { rev: 0, updatedAt: 0 };
          highWater.set(id, {
            rev: Math.max(cur.rev, entry.rev),
            updatedAt: Math.max(cur.updatedAt, entry.updatedAt),
          });
        }
      },
      writeOutbox: writeChannelMutesOutbox,
      clearOutbox: clearChannelMutesOutbox,
      isLocalNonEmpty: (s) => Object.keys(s.channels).length > 0,
    });
  }

  /** Publish a mute store, debounced to 2s. */
  publishMutes(store: ChannelMuteStore, preservedKey?: string): void {
    this.publish(store, preservedKey);
  }

  /**
   * Re-drive the current pending mute edit without resetting the preserved key
   * — used by the reconnect handler (Kalvin P3).
   */
  retryReconnectMutesPublish(): void {
    this.retryReconnectPublish();
  }

  /** Fetch the current remote head for this pubkey's mutes blob. */
  fetchRemoteMutes() {
    return this.fetchRemoteBlob();
  }

  /** Cancel any pending mute publish (debounce or retry timer). */
  cancelPendingMutePublish(): void {
    this.cancelPendingPublish();
  }

  /** The currently pending store, or null if nothing is queued. */
  getPendingMuteStore(): ChannelMuteStore | null {
    return this.getPendingStore();
  }

  /** Subscribe to live relay events for this pubkey's mutes blob. */
  subscribeToMutes(onUpdate: (remote: RemoteMutes) => void) {
    return this.subscribeLive(onUpdate);
  }
}
