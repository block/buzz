import { relayClient } from "@/shared/api/relayClient";
import {
  nip44DecryptFromSelf,
  nip44EncryptToSelf,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_CHANNEL_SORT } from "@/shared/constants/kinds";
import {
  parseChannelSortPayload,
  type ChannelSortStore,
} from "./channelSortPreference";

const D_TAG = "channel-sort";
const DEBOUNCE_MS = 2_000;

export type RemoteSortPrefs = {
  store: ChannelSortStore;
  createdAt: number;
  eventId: string;
};

async function decryptAndParse(
  event: RelayEvent,
): Promise<RemoteSortPrefs | null> {
  try {
    const plaintext = await nip44DecryptFromSelf(event.content);
    const store = parseChannelSortPayload(JSON.parse(plaintext));
    if (!store) return null;
    return { store, createdAt: event.created_at, eventId: event.id };
  } catch {
    return null;
  }
}

/**
 * Syncs the per-group sidebar sort preferences across clients via encrypted
 * NIP-78 app data (kind 30078, d-tag `channel-sort`), following the same
 * pattern as channel sections: NIP-44 encrypted-to-self content, debounced
 * writes, and whole-blob last-write-wins. The sort map is a compact,
 * low-frequency preference blob, so whole-blob LWW (like sections) is
 * sufficient — per-key merge (like stars/mutes) would be unnecessary
 * complexity here.
 */
export class ChannelSortSyncManager {
  private pubkey: string;
  private debounceTimer: number | null = null;
  private lastRemoteCreatedAt = 0;
  private pendingStore: ChannelSortStore | null = null;
  private lastPublishedStore: ChannelSortStore | null = null;
  private destroyed = false;

  constructor(pubkey: string) {
    this.pubkey = pubkey;
  }

  async fetchRemoteSortPrefs(): Promise<RemoteSortPrefs | null> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_CHANNEL_SORT],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 1,
      });
      if (events.length === 0) return null;
      if (events[0].pubkey !== this.pubkey) return null;
      const result = await decryptAndParse(events[0]);
      if (result) {
        this.lastRemoteCreatedAt = Math.max(
          this.lastRemoteCreatedAt,
          result.createdAt,
        );
      }
      return result;
    } catch {
      return null;
    }
  }

  cancelPendingPublish(): void {
    if (this.debounceTimer !== null) {
      window.clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
  }

  getPendingStore(): ChannelSortStore | null {
    return this.pendingStore;
  }

  publishSortPrefs(store: ChannelSortStore): void {
    this.pendingStore = store;
    if (this.debounceTimer !== null) {
      window.clearTimeout(this.debounceTimer);
    }
    this.debounceTimer = window.setTimeout(() => {
      this.debounceTimer = null;
      void this.doPublish(store);
    }, DEBOUNCE_MS);
  }

  private async fetchOwnBlobBeforePublish(
    store: ChannelSortStore,
  ): Promise<ChannelSortStore> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_CHANNEL_SORT],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 1,
      });
      if (events.length === 0 || events[0].pubkey !== this.pubkey) return store;
      const remote = await decryptAndParse(events[0]);
      if (!remote) return store;
      // Sort prefs use whole-blob LWW: take whichever is newer
      if (remote.createdAt > this.lastRemoteCreatedAt) {
        this.lastRemoteCreatedAt = remote.createdAt;
        return remote.store;
      }
      return store;
    } catch {
      return store;
    }
  }

  private isIdenticalToLastPublished(store: ChannelSortStore): boolean {
    if (!this.lastPublishedStore) return false;
    const lastGroups = this.lastPublishedStore.groups;
    const currentGroups = store.groups;
    const lastKeys = Object.keys(lastGroups);
    const currentKeys = Object.keys(currentGroups);
    if (lastKeys.length !== currentKeys.length) return false;
    for (const key of currentKeys) {
      if (lastGroups[key] !== currentGroups[key]) return false;
    }
    return true;
  }

  private async doPublish(store: ChannelSortStore): Promise<void> {
    try {
      const merged = await this.fetchOwnBlobBeforePublish(store);
      // Guard: manager may have been destroyed while fetchOwnBlobBeforePublish
      // was awaited (workspace switch during in-flight fetch). If so, abort
      // before touching the relay.
      if (this.destroyed) return;
      if (this.isIdenticalToLastPublished(merged)) {
        this.pendingStore = null;
        return;
      }
      const payload = {
        version: 1,
        groups: merged.groups,
      };
      const ciphertext = await nip44EncryptToSelf(JSON.stringify(payload));
      const createdAt = Math.max(
        Math.floor(Date.now() / 1_000),
        this.lastRemoteCreatedAt + 1,
      );
      const event = await signRelayEvent({
        kind: KIND_CHANNEL_SORT,
        content: ciphertext,
        createdAt,
        tags: [
          ["d", D_TAG],
          ["t", D_TAG], // relay discoverability; not used in our filters
        ],
      });
      // Final guard immediately before the network call — sign/encrypt are
      // synchronous-ish but cheap; the relay socket may have moved to a
      // different workspace by the time we reach this point.
      if (this.destroyed) return;
      await relayClient.publishEvent(
        event,
        "Timed out publishing channel sort preferences.",
        "Failed to publish channel sort preferences.",
      );
      this.lastRemoteCreatedAt = Math.max(
        this.lastRemoteCreatedAt,
        event.created_at,
      );
      this.lastPublishedStore = merged;
      this.pendingStore = null;
    } catch (error) {
      console.warn("[channelSortSync] publish failed:", error);
    }
  }

  async subscribeToSortPrefs(
    onUpdate: (remote: RemoteSortPrefs) => void,
  ): Promise<() => Promise<void>> {
    return relayClient.subscribeLive(
      {
        kinds: [KIND_CHANNEL_SORT],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 0,
      },
      (event: RelayEvent) => {
        if (event.pubkey !== this.pubkey) return;
        void decryptAndParse(event).then((result) => {
          if (result) {
            this.lastRemoteCreatedAt = Math.max(
              this.lastRemoteCreatedAt,
              result.createdAt,
            );
            onUpdate(result);
          }
        });
      },
    );
  }

  destroy(): void {
    // Cancel any pending publish and mark this manager as destroyed so any
    // in-flight doPublish() calls abort before reaching relayClient. The
    // scoped localStorage write is already durable; when the user returns to
    // this relay the existing seed-publish guard will re-publish from local
    // state. Flushing here would race against workspace switching and could
    // publish relay A's sort prefs to relay B via the shared relayClient
    // singleton.
    this.destroyed = true;
    this.cancelPendingPublish();
    this.pendingStore = null;
  }
}
