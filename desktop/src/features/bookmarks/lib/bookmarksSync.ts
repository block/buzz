import { relayClient } from "@/shared/api/relayClient";
import {
  nip44DecryptFromSelf,
  nip44EncryptToSelf,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_BOOKMARKS } from "@/shared/constants/kinds";
import {
  mergeStores,
  parseBookmarkPayload,
  type BookmarkStore,
} from "./bookmarksStorage";

const D_TAG = "bookmarks";
const DEBOUNCE_MS = 2_000;

export type RemoteBookmarks = {
  store: BookmarkStore;
  createdAt: number;
  eventId: string;
};

async function decryptAndParse(
  event: RelayEvent,
): Promise<RemoteBookmarks | null> {
  try {
    const plaintext = await nip44DecryptFromSelf(event.content);
    const store = parseBookmarkPayload(JSON.parse(plaintext));
    if (!store) return null;
    return { store, createdAt: event.created_at, eventId: event.id };
  } catch {
    return null;
  }
}

/**
 * Per-user private bookmark list synced as an encrypted NIP-78 (kind 30078)
 * replaceable event, d-tag "bookmarks". Structural clone of
 * `ChannelStarSyncManager` — debounced encrypted publish with a
 * fetch-merge-before-publish guard so concurrent devices don't clobber, plus a
 * live subscription for cross-device updates.
 */
export class BookmarkSyncManager {
  private pubkey: string;
  private debounceTimer: number | null = null;
  private lastRemoteCreatedAt = 0;
  private pendingStore: BookmarkStore | null = null;
  private lastPublishedStore: BookmarkStore | null = null;

  constructor(pubkey: string) {
    this.pubkey = pubkey;
  }

  async fetchRemoteBookmarks(): Promise<RemoteBookmarks | null> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_BOOKMARKS],
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

  cancelPendingBookmarkPublish(): void {
    if (this.debounceTimer !== null) {
      window.clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
  }

  getPendingBookmarkStore(): BookmarkStore | null {
    return this.pendingStore;
  }

  publishBookmarks(store: BookmarkStore): void {
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
    store: BookmarkStore,
  ): Promise<BookmarkStore> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_BOOKMARKS],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 1,
      });
      if (events.length === 0 || events[0].pubkey !== this.pubkey) return store;
      const remote = await decryptAndParse(events[0]);
      if (!remote) return store;
      this.lastRemoteCreatedAt = Math.max(
        this.lastRemoteCreatedAt,
        remote.createdAt,
      );
      return mergeStores(store, remote.store);
    } catch {
      return store;
    }
  }

  private isIdenticalToLastPublished(store: BookmarkStore): boolean {
    if (!this.lastPublishedStore) return false;
    const lastKeys = Object.keys(this.lastPublishedStore.bookmarks);
    const currentKeys = Object.keys(store.bookmarks);
    if (lastKeys.length !== currentKeys.length) return false;
    for (const key of currentKeys) {
      const last = this.lastPublishedStore.bookmarks[key];
      const current = store.bookmarks[key];
      if (
        !last ||
        last.bookmarked !== current.bookmarked ||
        last.updatedAt !== current.updatedAt
      )
        return false;
    }
    return true;
  }

  private async doPublish(store: BookmarkStore): Promise<void> {
    try {
      const merged = await this.fetchOwnBlobBeforePublish(store);
      if (this.isIdenticalToLastPublished(merged)) {
        this.pendingStore = null;
        return;
      }
      const payload = {
        version: 1,
        bookmarks: merged.bookmarks,
      };
      const ciphertext = await nip44EncryptToSelf(JSON.stringify(payload));
      const createdAt = Math.max(
        Math.floor(Date.now() / 1_000),
        this.lastRemoteCreatedAt + 1,
      );
      const event = await signRelayEvent({
        kind: KIND_BOOKMARKS,
        content: ciphertext,
        createdAt,
        tags: [
          ["d", D_TAG],
          ["t", D_TAG], // relay discoverability; not used in our filters
        ],
      });
      await relayClient.publishEvent(
        event,
        "Timed out publishing bookmarks.",
        "Failed to publish bookmarks.",
      );
      this.lastRemoteCreatedAt = Math.max(
        this.lastRemoteCreatedAt,
        event.created_at,
      );
      this.lastPublishedStore = merged;
      this.pendingStore = null;
    } catch (error) {
      console.warn("[bookmarksSync] publish failed:", error);
    }
  }

  async subscribeToBookmarks(
    onUpdate: (remote: RemoteBookmarks) => void,
  ): Promise<() => Promise<void>> {
    return relayClient.subscribeLive(
      {
        kinds: [KIND_BOOKMARKS],
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
    if (this.debounceTimer !== null && this.pendingStore !== null) {
      window.clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
      void this.doPublish(this.pendingStore);
    } else if (this.debounceTimer !== null) {
      window.clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
  }
}
