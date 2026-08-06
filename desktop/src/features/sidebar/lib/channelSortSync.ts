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
import {
  advanceWatermark,
  readWatermark,
  runBootstrap,
  type FetchResult,
} from "./sidebarSyncWatermark";

const D_TAG = "channel-sort";
const BLOB_TYPE = D_TAG;
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
  private relayUrl: string;
  private debounceTimer: number | null = null;
  private lastRemoteCreatedAt: number;
  private pendingStore: ChannelSortStore | null = null;
  private lastPublishedStore: ChannelSortStore | null = null;
  private destroyed = false;

  constructor(pubkey: string, relayUrl: string) {
    this.pubkey = pubkey;
    this.relayUrl = relayUrl;
    this.lastRemoteCreatedAt = readWatermark(pubkey, BLOB_TYPE, relayUrl);
  }

  async fetchRemoteSortPrefs(): Promise<FetchResult<RemoteSortPrefs>> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_CHANNEL_SORT],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 1,
      });
      if (events.length === 0 || events[0].pubkey !== this.pubkey) {
        return { status: "absent" };
      }
      const event = events[0];
      this.recordRemoteHead(event.created_at);
      const result = await decryptAndParse(event);
      if (!result) {
        return { status: "failed", createdAt: event.created_at };
      }
      return {
        status: "found",
        data: result,
        createdAt: result.createdAt,
        eventId: result.eventId,
      };
    } catch {
      return { status: "failed" };
    }
  }

  private recordRemoteHead(createdAt: number): void {
    if (createdAt > this.lastRemoteCreatedAt) {
      this.lastRemoteCreatedAt = createdAt;
    }
    advanceWatermark(this.pubkey, BLOB_TYPE, this.relayUrl, createdAt);
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
      const event = events[0];
      // Snapshot the watermark before advancing it: after recordRemoteHead
      // runs, lastRemoteCreatedAt equals event.created_at, so the LWW
      // comparison remote.createdAt > lastRemoteCreatedAt would always be
      // false and silently suppress the merge.
      const headBeforeFetch = this.lastRemoteCreatedAt;
      this.recordRemoteHead(event.created_at);
      const remote = await decryptAndParse(event);
      if (!remote) return store;
      // Sort prefs use whole-blob LWW: take whichever is newer
      if (remote.createdAt > headBeforeFetch) {
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
      // was awaited (community switch during in-flight fetch). If so, abort
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
      // different community by the time we reach this point.
      if (this.destroyed) return;
      await relayClient.publishEvent(
        event,
        "Timed out publishing channel sort preferences.",
        "Failed to publish channel sort preferences.",
      );
      this.recordRemoteHead(event.created_at);
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
        // Record the raw head before decrypt so an undecryptable live event
        // still advances the watermark and blocks future seed-publish.
        this.recordRemoteHead(event.created_at);
        void decryptAndParse(event).then((result) => {
          if (result) {
            onUpdate(result);
          }
        });
      },
    );
  }

  /**
   * Fetches the remote blob on first mount, records the remote head, and
   * delegates the seed/hold/apply-remote decision to `runBootstrap`.
   */
  async bootstrap(localStore: ChannelSortStore) {
    const fetchResult = await this.fetchRemoteSortPrefs();
    return runBootstrap({
      fetchResult,
      lastHead: this.lastRemoteCreatedAt,
      localStore,
      isLocalNonEmpty: (s) => Object.keys(s.groups).length > 0,
      publishFn: (s) => this.publishSortPrefs(s),
    });
  }

  destroy(): void {
    // Cancel any pending publish and mark this manager as destroyed so any
    // in-flight doPublish() calls abort before reaching relayClient.
    // Pending debounce-window changes are intentionally dropped: flushing
    // could publish relay A's sort prefs to relay B via the shared relayClient
    // singleton. On return, bootstrap's found path whole-blob-replaces from
    // remote, so any dropped pending edit is lost.
    this.destroyed = true;
    this.cancelPendingPublish();
    this.pendingStore = null;
  }
}
