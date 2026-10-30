import { relayClient } from "@/shared/api/relayClient";
import {
  nip44DecryptFromSelf,
  nip44EncryptToSelf,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_CHANNEL_MUTES } from "@/shared/constants/kinds";
import {
  mergeStores,
  parseMutePayload,
  type ChannelMuteStore,
} from "./channelMutesStorage";
import {
  advanceWatermark,
  readWatermark,
  runBootstrap,
  type FetchResult,
} from "./sidebarSyncWatermark";

const D_TAG = "channel-mutes";
const BLOB_TYPE = D_TAG;
const DEBOUNCE_MS = 2_000;

export type RemoteMutes = {
  store: ChannelMuteStore;
  createdAt: number;
  eventId: string;
};

async function decryptAndParse(event: RelayEvent): Promise<RemoteMutes | null> {
  try {
    const plaintext = await nip44DecryptFromSelf(event.content);
    const store = parseMutePayload(JSON.parse(plaintext));
    if (!store) return null;
    return { store, createdAt: event.created_at, eventId: event.id };
  } catch {
    return null;
  }
}

export class ChannelMuteSyncManager {
  private pubkey: string;
  private relayUrl: string;
  private debounceTimer: number | null = null;
  private lastRemoteCreatedAt: number;
  private pendingStore: ChannelMuteStore | null = null;
  private lastPublishedStore: ChannelMuteStore | null = null;
  /** The bootstrap first-copy seed; retired once any relay head is observed. */
  private seedStore: ChannelMuteStore | null = null;
  /** Decoded heads, LWW-merged within the 500-entry bound, for each attempt. */
  private remoteStore: ChannelMuteStore | null = null;
  private remoteRev = 0;
  private destroyed = false;

  constructor(pubkey: string, relayUrl: string) {
    this.pubkey = pubkey;
    this.relayUrl = relayUrl;
    this.lastRemoteCreatedAt = readWatermark(pubkey, BLOB_TYPE, relayUrl);
  }

  async fetchRemoteMutes(): Promise<FetchResult<RemoteMutes>> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_CHANNEL_MUTES],
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
      this.remember(result.store);
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
    // A seed only fills a relay believed empty; any head supersedes it, even
    // one whose payload a concurrent reader drops or cannot decrypt.
    if (this.seedStore && this.pendingStore === this.seedStore) {
      this.cancelPendingMutePublish();
      this.pendingStore = null;
    }
    this.seedStore = null;
  }

  private remember(store: ChannelMuteStore): void {
    const next = this.remoteStore
      ? mergeStores(this.remoteStore, store)
      : store;
    // Only a changed winner invalidates in-flight attempts.
    if (!this.sameEntries(this.remoteStore, next)) this.remoteRev++;
    this.remoteStore = next;
  }

  cancelPendingMutePublish(): void {
    if (this.debounceTimer !== null) {
      window.clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
  }

  getPendingMuteStore(): ChannelMuteStore | null {
    return this.pendingStore;
  }

  publishMutes(store: ChannelMuteStore): void {
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
    store: ChannelMuteStore,
  ): Promise<ChannelMuteStore> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_CHANNEL_MUTES],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 1,
      });
      if (events.length === 0 || events[0].pubkey !== this.pubkey) return store;
      const event = events[0];
      // Record the raw head before decrypt on the pre-publish path too.
      this.recordRemoteHead(event.created_at);
      const remote = await decryptAndParse(event);
      if (!remote) return store;
      this.remember(remote.store);
      return store;
    } catch {
      return store;
    }
  }

  /** Same winners (IDs, `muted`, `updatedAt`), ignoring key order. */
  private sameEntries(prev: ChannelMuteStore | null, store: ChannelMuteStore) {
    if (!prev) return false;
    const lastKeys = Object.keys(prev.channels);
    const currentKeys = Object.keys(store.channels);
    if (lastKeys.length !== currentKeys.length) return false;
    for (const key of currentKeys) {
      const last = prev.channels[key];
      const current = store.channels[key];
      if (
        !last ||
        last.muted !== current.muted ||
        last.updatedAt !== current.updatedAt
      )
        return false;
    }
    return true;
  }

  private async doPublish(store: ChannelMuteStore): Promise<void> {
    // A seed retired while acquired (preflight, crypto or socket wait) aborts.
    const seed = store === this.seedStore;
    let rev = this.remoteRev; // a changed winner before dispatch voids it
    const stale = () =>
      this.destroyed ||
      (seed && this.seedStore !== store) ||
      rev !== this.remoteRev;
    // Only the attempt that still owns `pendingStore` may clear it.
    const release = () => {
      if (this.pendingStore === store) this.pendingStore = null;
    };
    try {
      const pre = await this.fetchOwnBlobBeforePublish(store);
      const merged = this.remoteStore
        ? mergeStores(pre, this.remoteStore)
        : pre;
      rev = this.remoteRev;
      // Guard: manager may have been destroyed while fetchOwnBlobBeforePublish
      // was awaited (community switch during in-flight fetch). If so, abort
      // before touching the relay.
      if (stale()) return;
      if (this.sameEntries(this.lastPublishedStore, merged)) {
        release();
        return;
      }
      const payload = {
        version: 1,
        channels: merged.channels,
      };
      const ciphertext = await nip44EncryptToSelf(JSON.stringify(payload));
      const createdAt = Math.max(
        Math.floor(Date.now() / 1_000),
        this.lastRemoteCreatedAt + 1,
      );
      const event = await signRelayEvent({
        kind: KIND_CHANNEL_MUTES,
        content: ciphertext,
        createdAt,
        tags: [
          ["d", D_TAG],
          ["t", D_TAG], // relay discoverability; not used in our filters
        ],
      });
      if (stale()) return;
      await relayClient.publishEvent(
        event,
        "Timed out publishing channel mutes.",
        "Failed to publish channel mutes.",
        () => !stale(),
      );
      this.recordRemoteHead(event.created_at);
      this.lastPublishedStore = merged;
      release();
    } catch (error) {
      console.warn("[channelMutesSync] publish failed:", error);
    } finally {
      const owned = !this.destroyed && this.pendingStore === store;
      if (owned && rev !== this.remoteRev) this.publishMutes(store);
    }
  }

  async subscribeToMutes(
    onUpdate: (remote: RemoteMutes) => void,
  ): Promise<() => Promise<void>> {
    return relayClient.subscribeLive(
      {
        kinds: [KIND_CHANNEL_MUTES],
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
          if (result && !this.destroyed) {
            this.remember(result.store);
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
  async bootstrap(localStore: ChannelMuteStore) {
    const fetchResult = await this.fetchRemoteMutes();
    return runBootstrap({
      fetchResult,
      lastHead: this.lastRemoteCreatedAt,
      localStore,
      isLocalNonEmpty: (s) => Object.keys(s.channels).length > 0,
      publishFn: (s) => {
        this.seedStore = s;
        this.publishMutes(s);
      },
    });
  }

  destroy(): void {
    // Cancel any pending publish and mark this manager as destroyed so any
    // in-flight doPublish() calls abort before reaching relayClient.
    // Pending debounce-window changes are intentionally dropped: flushing
    // could publish relay A's state to relay B via the shared relayClient
    // singleton. Local entries survive because the apply/publish paths merge
    // per-entry via mergeStores, so no local work is permanently lost.
    this.destroyed = true;
    this.cancelPendingMutePublish();
    this.pendingStore = null;
  }
}
