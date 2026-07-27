import { relayClient } from "@/shared/api/relayClient";
import {
  nip44DecryptFromSelf,
  nip44EncryptToSelf,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_CHANNEL_NOTIFY_PREFS } from "@/shared/constants/kinds";
import {
  mergeStores,
  parseNotifyPrefsPayload,
  storesEqual,
  type ChannelNotifyPrefsStore,
} from "./channelNotifyPrefsStorage";

const D_TAG = "channel-notify-prefs";
const DEBOUNCE_MS = 2_000;

export type RemoteNotifyPrefs = {
  store: ChannelNotifyPrefsStore;
  createdAt: number;
  eventId: string;
};

async function decryptAndParse(
  event: RelayEvent,
): Promise<RemoteNotifyPrefs | null> {
  try {
    const plaintext = await nip44DecryptFromSelf(event.content);
    const store = parseNotifyPrefsPayload(JSON.parse(plaintext));
    if (!store) return null;
    return { store, createdAt: event.created_at, eventId: event.id };
  } catch {
    return null;
  }
}

/**
 * Syncs per-channel notification preferences across a user's clients via
 * encrypted NIP-78 app data (kind 30078, d-tag `channel-notify-prefs`, NIP-44
 * encrypted to self). Writes are debounced and merged per channel with the
 * user's own remote blob before publishing (max-`updatedAt` LWW), so a stale
 * device cannot erase a newer entry from another device. See docs/nips/NIP-CN.md.
 */
export class ChannelNotifyPrefsSyncManager {
  private pubkey: string;
  private debounceTimer: number | null = null;
  private lastRemoteCreatedAt = 0;
  private pendingStore: ChannelNotifyPrefsStore | null = null;
  private lastPublishedStore: ChannelNotifyPrefsStore | null = null;
  private destroyed = false;

  constructor(pubkey: string) {
    this.pubkey = pubkey;
  }

  private async fetchOwnEvent(): Promise<RemoteNotifyPrefs | null> {
    const events = await relayClient.fetchEvents({
      kinds: [KIND_CHANNEL_NOTIFY_PREFS],
      authors: [this.pubkey],
      "#d": [D_TAG],
      limit: 1,
    });
    if (events.length === 0 || events[0].pubkey !== this.pubkey) return null;
    return decryptAndParse(events[0]);
  }

  async fetchRemotePrefs(): Promise<RemoteNotifyPrefs | null> {
    try {
      const result = await this.fetchOwnEvent();
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

  getPendingStore(): ChannelNotifyPrefsStore | null {
    return this.pendingStore;
  }

  publishPrefs(store: ChannelNotifyPrefsStore): void {
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
    store: ChannelNotifyPrefsStore,
  ): Promise<ChannelNotifyPrefsStore> {
    try {
      const remote = await this.fetchOwnEvent();
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

  private async doPublish(store: ChannelNotifyPrefsStore): Promise<void> {
    try {
      const merged = await this.fetchOwnBlobBeforePublish(store);
      // The manager may have been destroyed while the fetch was awaited
      // (community switch mid-flight) — never publish relay A's prefs to relay B
      // through the shared relayClient singleton.
      if (this.destroyed) return;
      if (
        this.lastPublishedStore &&
        storesEqual(this.lastPublishedStore, merged)
      ) {
        this.pendingStore = null;
        return;
      }
      // Size seam: NIP-44 rejects plaintext over 65,535 bytes and the catch
      // below only warns, so an oversized blob silently stops syncing while
      // local state keeps working. Sparse entries keep the ceiling at several
      // hundred *customized* channels. Shared with the four sibling kind-30078
      // sidebar blobs; a common pre-encrypt budget is tracked separately.
      const ciphertext = await nip44EncryptToSelf(
        JSON.stringify({ version: 1, channels: merged.channels }),
      );
      const createdAt = Math.max(
        Math.floor(Date.now() / 1_000),
        this.lastRemoteCreatedAt + 1,
      );
      const event = await signRelayEvent({
        kind: KIND_CHANNEL_NOTIFY_PREFS,
        content: ciphertext,
        createdAt,
        tags: [
          ["d", D_TAG],
          ["t", D_TAG], // relay discoverability; not used in our filters
        ],
      });
      if (this.destroyed) return;
      await relayClient.publishEvent(
        event,
        "Timed out publishing channel notification preferences.",
        "Failed to publish channel notification preferences.",
      );
      this.lastRemoteCreatedAt = Math.max(
        this.lastRemoteCreatedAt,
        event.created_at,
      );
      this.lastPublishedStore = merged;
      this.pendingStore = null;
    } catch (error) {
      console.warn("[channelNotifyPrefsSync] publish failed:", error);
    }
  }

  async subscribeToPrefs(
    onUpdate: (remote: RemoteNotifyPrefs) => void,
  ): Promise<() => Promise<void>> {
    return relayClient.subscribeLive(
      {
        kinds: [KIND_CHANNEL_NOTIFY_PREFS],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 0,
      },
      (event: RelayEvent) => {
        if (event.pubkey !== this.pubkey) return;
        void decryptAndParse(event).then((result) => {
          if (!result) return;
          this.lastRemoteCreatedAt = Math.max(
            this.lastRemoteCreatedAt,
            result.createdAt,
          );
          onUpdate(result);
        });
      },
    );
  }

  destroy(): void {
    // Cancel rather than flush: the relay-scoped localStorage write is already
    // durable and the hook re-publishes pending edits when the user returns to
    // this relay. Flushing here races community switching (see #1556).
    this.destroyed = true;
    this.cancelPendingPublish();
    this.pendingStore = null;
  }
}
