import { relayClient } from "@/shared/api/relayClient";
import {
  nip44DecryptFromSelf,
  nip44EncryptToSelf,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_CHANNEL_MANUAL_ORDER } from "@/shared/constants/kinds";
import {
  parseChannelManualOrderPayload,
  type ChannelManualOrderStore,
} from "./channelManualOrder";

const D_TAG = "channel-manual-order";
const DEBOUNCE_MS = 2_000;

export type RemoteManualOrder = {
  store: ChannelManualOrderStore;
  createdAt: number;
  eventId: string;
};

async function decryptAndParse(
  event: RelayEvent,
): Promise<RemoteManualOrder | null> {
  try {
    const plaintext = await nip44DecryptFromSelf(event.content);
    const store = parseChannelManualOrderPayload(JSON.parse(plaintext));
    if (!store) return null;
    return { store, createdAt: event.created_at, eventId: event.id };
  } catch {
    return null;
  }
}

export class ChannelManualOrderSyncManager {
  private pubkey: string;
  private debounceTimer: number | null = null;
  private lastRemoteCreatedAt = 0;
  private pendingStore: ChannelManualOrderStore | null = null;
  private lastPublishedJson = "";
  private destroyed = false;

  constructor(pubkey: string) {
    this.pubkey = pubkey;
  }

  async fetchRemote(): Promise<RemoteManualOrder | null> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_CHANNEL_MANUAL_ORDER],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 1,
      });
      if (events.length === 0 || events[0].pubkey !== this.pubkey) return null;
      const result = await decryptAndParse(events[0]);
      if (result) {
        this.shouldApplyRemote(result);
      }
      return result;
    } catch {
      return null;
    }
  }

  publish(store: ChannelManualOrderStore): void {
    this.pendingStore = store;
    this.cancelPendingTimer();
    this.debounceTimer = window.setTimeout(() => {
      this.debounceTimer = null;
      void this.doPublish(store);
    }, DEBOUNCE_MS);
  }

  private cancelPendingTimer(): void {
    if (this.debounceTimer !== null) {
      window.clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
  }

  discardPendingPublish(): void {
    this.cancelPendingTimer();
    this.pendingStore = null;
  }

  getPendingStore(): ChannelManualOrderStore | null {
    return this.pendingStore;
  }

  shouldApplyRemote(remote: RemoteManualOrder): boolean {
    this.lastRemoteCreatedAt = Math.max(
      this.lastRemoteCreatedAt,
      remote.createdAt,
    );
    return this.pendingStore === null;
  }

  private async refreshRemoteTimestampBeforePublish(): Promise<void> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_CHANNEL_MANUAL_ORDER],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 1,
      });
      if (events.length === 0 || events[0].pubkey !== this.pubkey) return;
      const remote = await decryptAndParse(events[0]);
      if (!remote) return;
      this.lastRemoteCreatedAt = Math.max(
        this.lastRemoteCreatedAt,
        remote.createdAt,
      );
    } catch {
      // Publishing the explicit local edit is still safe: the event timestamp
      // below is monotonic against every remote value this manager has seen.
    }
  }

  private async doPublish(store: ChannelManualOrderStore): Promise<void> {
    try {
      await this.refreshRemoteTimestampBeforePublish();
      if (this.destroyed) return;
      const json = JSON.stringify(store);
      if (json === this.lastPublishedJson) {
        this.pendingStore = null;
        return;
      }
      const ciphertext = await nip44EncryptToSelf(json);
      const createdAt = Math.max(
        Math.floor(Date.now() / 1_000),
        this.lastRemoteCreatedAt + 1,
      );
      const event = await signRelayEvent({
        kind: KIND_CHANNEL_MANUAL_ORDER,
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
        "Timed out publishing manual channel order.",
        "Failed to publish manual channel order.",
      );
      this.lastRemoteCreatedAt = event.created_at;
      this.lastPublishedJson = json;
      this.pendingStore = null;
    } catch (error) {
      console.warn("[channelManualOrderSync] publish failed:", error);
    }
  }

  async subscribe(
    onUpdate: (remote: RemoteManualOrder) => void,
  ): Promise<() => Promise<void>> {
    return relayClient.subscribeLive(
      {
        kinds: [KIND_CHANNEL_MANUAL_ORDER],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 0,
      },
      (event: RelayEvent) => {
        if (event.pubkey !== this.pubkey) return;
        void decryptAndParse(event).then((result) => {
          if (!result) return;
          this.shouldApplyRemote(result);
          onUpdate(result);
        });
      },
    );
  }

  destroy(): void {
    this.destroyed = true;
    this.discardPendingPublish();
  }
}
