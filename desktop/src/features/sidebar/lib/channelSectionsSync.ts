import { relayClient } from "@/shared/api/relayClient";
import {
  nip44DecryptFromSelf,
  nip44EncryptToSelf,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_CHANNEL_SECTIONS } from "@/shared/constants/kinds";
import {
  parseChannelSectionPayload,
  type ChannelSection,
  type ChannelSectionStore,
} from "./channelSectionsStorage";

const D_TAG = "channel-sections";
const DEBOUNCE_MS = 2_000;

export type RemoteSections = {
  store: ChannelSectionStore;
  createdAt: number;
  eventId: string;
};

/** NIP-78 plaintext shape. Optional channelsBlockIndex keeps v1 backward-readable. */
export function serializeChannelSectionsPayload(store: ChannelSectionStore): {
  version: 1;
  sections: ChannelSection[];
  assignments: Record<string, string>;
  channelsBlockIndex?: number;
} {
  return {
    version: 1,
    sections: store.sections,
    assignments: store.assignments,
    ...(typeof store.channelsBlockIndex === "number"
      ? { channelsBlockIndex: store.channelsBlockIndex }
      : {}),
  };
}

/** Equality used to skip no-op re-publishes (includes index-only moves). */
export function channelSectionStoresEqual(
  left: ChannelSectionStore,
  right: ChannelSectionStore,
): boolean {
  if (left.sections.length !== right.sections.length) return false;
  for (let i = 0; i < right.sections.length; i++) {
    const a = left.sections[i] as ChannelSection | undefined;
    const b = right.sections[i] as ChannelSection;
    if (
      !a ||
      a.id !== b.id ||
      a.name !== b.name ||
      a.icon !== b.icon ||
      a.order !== b.order
    ) {
      return false;
    }
  }
  const leftKeys = Object.keys(left.assignments);
  const rightKeys = Object.keys(right.assignments);
  if (leftKeys.length !== rightKeys.length) return false;
  for (const key of rightKeys) {
    if (left.assignments[key] !== right.assignments[key]) return false;
  }
  return left.channelsBlockIndex === right.channelsBlockIndex;
}

async function decryptAndParse(
  event: RelayEvent,
): Promise<RemoteSections | null> {
  try {
    const plaintext = await nip44DecryptFromSelf(event.content);
    const store = parseChannelSectionPayload(JSON.parse(plaintext));
    if (!store) return null;
    return { store, createdAt: event.created_at, eventId: event.id };
  } catch {
    return null;
  }
}

export class ChannelSectionSyncManager {
  private pubkey: string;
  private debounceTimer: number | null = null;
  private lastRemoteCreatedAt = 0;
  private pendingStore: ChannelSectionStore | null = null;
  private lastPublishedStore: ChannelSectionStore | null = null;
  private destroyed = false;

  constructor(pubkey: string) {
    this.pubkey = pubkey;
  }

  async fetchRemoteSections(): Promise<RemoteSections | null> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_CHANNEL_SECTIONS],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 1,
      });
      if (events.length === 0) return null;
      if (events[0].pubkey !== this.pubkey) return null;
      const result = await decryptAndParse(events[0]);
      if (result) {
        this.shouldApplyRemote(result);
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

  getPendingStore(): ChannelSectionStore | null {
    return this.pendingStore;
  }

  shouldApplyRemote(remote: RemoteSections): boolean {
    this.lastRemoteCreatedAt = Math.max(
      this.lastRemoteCreatedAt,
      remote.createdAt,
    );
    return this.pendingStore === null;
  }

  publishSections(store: ChannelSectionStore): void {
    this.pendingStore = store;
    if (this.debounceTimer !== null) {
      window.clearTimeout(this.debounceTimer);
    }
    this.debounceTimer = window.setTimeout(() => {
      this.debounceTimer = null;
      void this.doPublish(store);
    }, DEBOUNCE_MS);
  }

  private async refreshRemoteTimestampBeforePublish(): Promise<void> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_CHANNEL_SECTIONS],
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

  private isIdenticalToLastPublished(store: ChannelSectionStore): boolean {
    if (!this.lastPublishedStore) return false;
    return channelSectionStoresEqual(this.lastPublishedStore, store);
  }

  private async doPublish(store: ChannelSectionStore): Promise<void> {
    try {
      await this.refreshRemoteTimestampBeforePublish();
      // Guard: manager may have been destroyed while the remote timestamp
      // was awaited (community switch during in-flight fetch). If so, abort
      // before touching the relay.
      if (this.destroyed) return;
      if (this.isIdenticalToLastPublished(store)) {
        this.pendingStore = null;
        return;
      }
      // Optional channelsBlockIndex keeps v1 payloads backward-readable:
      // older clients ignore the field; omit when unset so legacy layout wins.
      const payload = serializeChannelSectionsPayload(store);
      const ciphertext = await nip44EncryptToSelf(JSON.stringify(payload));
      const createdAt = Math.max(
        Math.floor(Date.now() / 1_000),
        this.lastRemoteCreatedAt + 1,
      );
      const event = await signRelayEvent({
        kind: KIND_CHANNEL_SECTIONS,
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
        "Timed out publishing channel sections.",
        "Failed to publish channel sections.",
      );
      this.lastRemoteCreatedAt = Math.max(
        this.lastRemoteCreatedAt,
        event.created_at,
      );
      this.lastPublishedStore = store;
      this.pendingStore = null;
    } catch (error) {
      console.warn("[channelSectionsSync] publish failed:", error);
    }
  }

  async subscribeToSections(
    onUpdate: (remote: RemoteSections) => void,
  ): Promise<() => Promise<void>> {
    return relayClient.subscribeLive(
      {
        kinds: [KIND_CHANNEL_SECTIONS],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 0,
      },
      (event: RelayEvent) => {
        if (event.pubkey !== this.pubkey) return;
        void decryptAndParse(event).then((result) => {
          if (result) {
            this.shouldApplyRemote(result);
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
    // state. Flushing here would race against community switching and could
    // publish relay A's sections to relay B via the shared relayClient
    // singleton.
    this.destroyed = true;
    this.cancelPendingPublish();
    this.pendingStore = null;
  }
}
