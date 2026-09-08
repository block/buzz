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
import {
  advanceWatermark,
  readWatermark,
  runBootstrap,
  type FetchResult,
} from "./sidebarSyncWatermark";

const D_TAG = "channel-sections";
const BLOB_TYPE = D_TAG;
const DEBOUNCE_MS = 2_000;

export type RemoteSections = {
  store: ChannelSectionStore;
  createdAt: number;
  eventId: string;
};

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
  private relayUrl: string;
  private debounceTimer: number | null = null;
  private lastRemoteCreatedAt: number;
  private pendingStore: ChannelSectionStore | null = null;
  private lastPublishedStore: ChannelSectionStore | null = null;
  private destroyed = false;

  constructor(pubkey: string, relayUrl: string) {
    this.pubkey = pubkey;
    this.relayUrl = relayUrl;
    // Hydrate from localStorage so we never seed-publish if a remote blob has
    // been seen in a prior session.
    this.lastRemoteCreatedAt = readWatermark(pubkey, BLOB_TYPE, relayUrl);
  }

  async fetchRemoteSections(): Promise<FetchResult<RemoteSections>> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_CHANNEL_SECTIONS],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 1,
      });
      if (events.length === 0 || events[0].pubkey !== this.pubkey) {
        return { status: "absent" };
      }
      const event = events[0];
      // An event exists — record its created_at regardless of whether we can
      // decrypt it, so seed-publish is blocked even when the payload is
      // unreadable (e.g. wrong key).
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

  /** Update in-memory + persisted watermark. */
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

  getPendingStore(): ChannelSectionStore | null {
    return this.pendingStore;
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

  private async fetchOwnBlobBeforePublish(
    store: ChannelSectionStore,
  ): Promise<ChannelSectionStore> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_CHANNEL_SECTIONS],
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
      // Sections use whole-blob LWW: take whichever is newer
      if (remote.createdAt > headBeforeFetch) {
        return remote.store;
      }
      return store;
    } catch {
      return store;
    }
  }

  private isIdenticalToLastPublished(store: ChannelSectionStore): boolean {
    if (!this.lastPublishedStore) return false;
    const lastSections = this.lastPublishedStore.sections;
    const currentSections = store.sections;
    if (lastSections.length !== currentSections.length) return false;
    for (let i = 0; i < currentSections.length; i++) {
      const last = lastSections[i] as ChannelSection | undefined;
      const current = currentSections[i] as ChannelSection;
      if (
        !last ||
        last.id !== current.id ||
        last.name !== current.name ||
        last.icon !== current.icon ||
        last.order !== current.order
      )
        return false;
    }
    const lastAssignKeys = Object.keys(this.lastPublishedStore.assignments);
    const currentAssignKeys = Object.keys(store.assignments);
    if (lastAssignKeys.length !== currentAssignKeys.length) return false;
    for (const key of currentAssignKeys) {
      if (this.lastPublishedStore.assignments[key] !== store.assignments[key])
        return false;
    }
    return true;
  }

  private async doPublish(store: ChannelSectionStore): Promise<void> {
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
        sections: merged.sections,
        assignments: merged.assignments,
      };
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
      this.recordRemoteHead(event.created_at);
      this.lastPublishedStore = merged;
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
  async bootstrap(localStore: ChannelSectionStore) {
    const fetchResult = await this.fetchRemoteSections();
    return runBootstrap({
      fetchResult,
      lastHead: this.lastRemoteCreatedAt,
      localStore,
      isLocalNonEmpty: (s) => s.sections.length > 0,
      publishFn: (s) => this.publishSections(s),
    });
  }

  destroy(): void {
    // Cancel any pending publish and mark this manager as destroyed so any
    // in-flight doPublish() calls abort before reaching relayClient.
    // Pending debounce-window changes are intentionally dropped: flushing
    // could publish relay A's sections to relay B via the shared relayClient
    // singleton. On return, bootstrap's found path whole-blob-replaces from
    // remote, so any dropped pending edit is lost.
    this.destroyed = true;
    this.cancelPendingPublish();
    this.pendingStore = null;
  }
}
