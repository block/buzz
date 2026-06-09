import { relayClient } from "@/shared/api/relayClient";
import {
  nip44DecryptFromSelf,
  nip44EncryptToSelf,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_CHANNEL_STARS } from "@/shared/constants/kinds";
import {
  mergeStores,
  parseStarPayload,
  type ChannelStarStore,
} from "./channelStarsStorage";

const D_TAG = "channel-stars";
const DEBOUNCE_MS = 2_000;

export type RemoteStars = {
  store: ChannelStarStore;
  createdAt: number;
  eventId: string;
};

async function decryptAndParse(event: RelayEvent): Promise<RemoteStars | null> {
  try {
    const plaintext = await nip44DecryptFromSelf(event.content);
    const store = parseStarPayload(JSON.parse(plaintext));
    if (!store) return null;
    return { store, createdAt: event.created_at, eventId: event.id };
  } catch {
    return null;
  }
}

export class ChannelStarSyncManager {
  private pubkey: string;
  private debounceTimer: number | null = null;
  private lastRemoteCreatedAt = 0;
  private pendingStore: ChannelStarStore | null = null;
  private lastPublishedStore: ChannelStarStore | null = null;

  constructor(pubkey: string) {
    this.pubkey = pubkey;
  }

  async fetchRemoteStars(): Promise<RemoteStars | null> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_CHANNEL_STARS],
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

  cancelPendingStarPublish(): void {
    if (this.debounceTimer !== null) {
      window.clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
  }

  getPendingStarStore(): ChannelStarStore | null {
    return this.pendingStore;
  }

  publishStars(store: ChannelStarStore): void {
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
    store: ChannelStarStore,
  ): Promise<ChannelStarStore> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_CHANNEL_STARS],
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

  private isIdenticalToLastPublished(store: ChannelStarStore): boolean {
    if (!this.lastPublishedStore) return false;
    const lastKeys = Object.keys(this.lastPublishedStore.channels);
    const currentKeys = Object.keys(store.channels);
    if (lastKeys.length !== currentKeys.length) return false;
    for (const key of currentKeys) {
      const last = this.lastPublishedStore.channels[key];
      const current = store.channels[key];
      if (
        !last ||
        last.starred !== current.starred ||
        last.updatedAt !== current.updatedAt
      )
        return false;
    }
    return true;
  }

  private async doPublish(store: ChannelStarStore): Promise<void> {
    try {
      const merged = await this.fetchOwnBlobBeforePublish(store);
      if (this.isIdenticalToLastPublished(merged)) {
        this.pendingStore = null;
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
        kind: KIND_CHANNEL_STARS,
        content: ciphertext,
        createdAt,
        tags: [
          ["d", D_TAG],
          ["t", D_TAG], // relay discoverability; not used in our filters
        ],
      });
      await relayClient.publishEvent(
        event,
        "Timed out publishing channel stars.",
        "Failed to publish channel stars.",
      );
      this.lastRemoteCreatedAt = Math.max(
        this.lastRemoteCreatedAt,
        event.created_at,
      );
      this.lastPublishedStore = merged;
      this.pendingStore = null;
    } catch (error) {
      console.warn("[channelStarsSync] publish failed:", error);
    }
  }

  async subscribeToStars(
    onUpdate: (remote: RemoteStars) => void,
  ): Promise<() => Promise<void>> {
    return relayClient.subscribeLive(
      {
        kinds: [KIND_CHANNEL_STARS],
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
