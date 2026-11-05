import { relayClient } from "@/shared/api/relayClient";
import {
  nip44DecryptFromSelf,
  nip44EncryptToSelf,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_THREAD_PINS } from "@/shared/constants/kinds";
import {
  advanceWatermark,
  readWatermark,
  runBootstrap,
  type FetchResult,
} from "@/features/sidebar/lib/sidebarSyncWatermark";
import {
  mergeThreadPinStores,
  parseThreadPinPayload,
  threadPinStoresEqual,
  type ThreadPinStore,
} from "./threadPins";

const D_TAG = "thread-pins";
const BLOB_TYPE = D_TAG;
const DEBOUNCE_MS = 2_000;

export type RemoteThreadPins = {
  store: ThreadPinStore;
  createdAt: number;
  eventId: string;
};

async function decryptAndParse(
  event: RelayEvent,
): Promise<RemoteThreadPins | null> {
  try {
    const plaintext = await nip44DecryptFromSelf(event.content);
    const store = parseThreadPinPayload(JSON.parse(plaintext));
    if (!store) return null;
    return { store, createdAt: event.created_at, eventId: event.id };
  } catch {
    return null;
  }
}

/** Syncs the user's pinned threads across devices; mirrors the project sidebar membership manager. */
export class ThreadPinsSyncManager {
  private pubkey: string;
  private relayUrl: string;
  private debounceTimer: number | null = null;
  private lastRemoteCreatedAt: number;
  private pendingStore: ThreadPinStore | null = null;
  private lastPublishedStore: ThreadPinStore | null = null;
  private destroyed = false;

  constructor(pubkey: string, relayUrl: string) {
    this.pubkey = pubkey;
    this.relayUrl = relayUrl;
    this.lastRemoteCreatedAt = readWatermark(pubkey, BLOB_TYPE, relayUrl);
  }

  private fetchOwnEvents() {
    return relayClient.fetchEvents({
      kinds: [KIND_THREAD_PINS],
      authors: [this.pubkey],
      "#d": [D_TAG],
      limit: 1,
    });
  }

  async fetchRemotePins(): Promise<FetchResult<RemoteThreadPins>> {
    try {
      const events = await this.fetchOwnEvents();
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

  getPendingStore(): ThreadPinStore | null {
    return this.pendingStore;
  }

  publishPins(store: ThreadPinStore): void {
    this.pendingStore = store;
    if (this.debounceTimer !== null) {
      window.clearTimeout(this.debounceTimer);
    }
    this.debounceTimer = window.setTimeout(() => {
      this.debounceTimer = null;
      void this.doPublish(store);
    }, DEBOUNCE_MS);
  }

  private async mergeWithRemoteBeforePublish(
    store: ThreadPinStore,
  ): Promise<ThreadPinStore> {
    try {
      const events = await this.fetchOwnEvents();
      if (events.length === 0 || events[0].pubkey !== this.pubkey) return store;
      const event = events[0];
      this.recordRemoteHead(event.created_at);
      const remote = await decryptAndParse(event);
      if (!remote) return store;
      return mergeThreadPinStores(store, remote.store);
    } catch {
      return store;
    }
  }

  private async doPublish(store: ThreadPinStore): Promise<void> {
    try {
      const merged = await this.mergeWithRemoteBeforePublish(store);
      if (this.destroyed) return;
      if (
        this.lastPublishedStore !== null &&
        threadPinStoresEqual(this.lastPublishedStore, merged)
      ) {
        this.pendingStore = null;
        return;
      }
      const ciphertext = await nip44EncryptToSelf(JSON.stringify(merged));
      const createdAt = Math.max(
        Math.floor(Date.now() / 1_000),
        this.lastRemoteCreatedAt + 1,
      );
      const event = await signRelayEvent({
        kind: KIND_THREAD_PINS,
        content: ciphertext,
        createdAt,
        tags: [
          ["d", D_TAG],
          ["t", D_TAG],
        ],
      });
      if (this.destroyed) return;
      await relayClient.publishEvent(
        event,
        "Timed out publishing pinned threads.",
        "Failed to publish pinned threads.",
      );
      this.recordRemoteHead(event.created_at);
      this.lastPublishedStore = merged;
      this.pendingStore = null;
    } catch (error) {
      console.warn("[threadPinsSync] publish failed:", error);
    }
  }

  async subscribe(
    onUpdate: (remote: RemoteThreadPins) => void,
  ): Promise<() => Promise<void>> {
    return relayClient.subscribeLive(
      {
        kinds: [KIND_THREAD_PINS],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 0,
      },
      (event: RelayEvent) => {
        if (event.pubkey !== this.pubkey) return;
        this.recordRemoteHead(event.created_at);
        void decryptAndParse(event).then((result) => {
          if (result) onUpdate(result);
        });
      },
    );
  }

  async bootstrap(localStore: ThreadPinStore) {
    const fetchResult = await this.fetchRemotePins();
    return runBootstrap({
      fetchResult,
      lastHead: this.lastRemoteCreatedAt,
      localStore,
      isLocalNonEmpty: (store) => Object.keys(store.roots).length > 0,
      publishFn: (store) => this.publishPins(store),
    });
  }

  destroy(): void {
    this.destroyed = true;
    this.cancelPendingPublish();
    this.pendingStore = null;
  }
}
