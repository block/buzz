import { relayClient } from "@/shared/api/relayClient";
import {
  nip44DecryptFromSelf,
  nip44EncryptToSelf,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_PROJECT_SIDEBAR_MEMBERSHIP } from "@/shared/constants/kinds";
import {
  mergeProjectSidebarMembershipStores,
  parseProjectSidebarMembershipPayload,
  projectSidebarMembershipStoresEqual,
  type ProjectSidebarMembershipStore,
} from "./projectSidebarMembership";
import {
  advanceWatermark,
  readWatermark,
  runBootstrap,
  type FetchResult,
} from "@/features/sidebar/lib/sidebarSyncWatermark";

const D_TAG = "project-sidebar-membership";
const BLOB_TYPE = D_TAG;
const DEBOUNCE_MS = 2_000;

export type RemoteProjectSidebarMembership = {
  store: ProjectSidebarMembershipStore;
  createdAt: number;
  eventId: string;
};

async function decryptAndParse(
  event: RelayEvent,
): Promise<RemoteProjectSidebarMembership | null> {
  try {
    const plaintext = await nip44DecryptFromSelf(event.content);
    const store = parseProjectSidebarMembershipPayload(JSON.parse(plaintext));
    if (!store) return null;
    return { store, createdAt: event.created_at, eventId: event.id };
  } catch {
    return null;
  }
}

export class ProjectSidebarMembershipSyncManager {
  private pubkey: string;
  private relayUrl: string;
  private debounceTimer: number | null = null;
  private lastRemoteCreatedAt: number;
  private pendingStore: ProjectSidebarMembershipStore | null = null;
  private lastPublishedStore: ProjectSidebarMembershipStore | null = null;
  private destroyed = false;

  constructor(pubkey: string, relayUrl: string) {
    this.pubkey = pubkey;
    this.relayUrl = relayUrl;
    this.lastRemoteCreatedAt = readWatermark(pubkey, BLOB_TYPE, relayUrl);
  }

  async fetchRemoteMembership(): Promise<
    FetchResult<RemoteProjectSidebarMembership>
  > {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_PROJECT_SIDEBAR_MEMBERSHIP],
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

  getPendingStore(): ProjectSidebarMembershipStore | null {
    return this.pendingStore;
  }

  publishMembership(store: ProjectSidebarMembershipStore): void {
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
    store: ProjectSidebarMembershipStore,
  ): Promise<ProjectSidebarMembershipStore> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_PROJECT_SIDEBAR_MEMBERSHIP],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 1,
      });
      if (events.length === 0 || events[0].pubkey !== this.pubkey) return store;
      const event = events[0];
      this.recordRemoteHead(event.created_at);
      const remote = await decryptAndParse(event);
      if (!remote) return store;
      return mergeProjectSidebarMembershipStores(store, remote.store);
    } catch {
      return store;
    }
  }

  private isIdenticalToLastPublished(
    store: ProjectSidebarMembershipStore,
  ): boolean {
    return (
      this.lastPublishedStore !== null &&
      projectSidebarMembershipStoresEqual(this.lastPublishedStore, store)
    );
  }

  private async doPublish(store: ProjectSidebarMembershipStore): Promise<void> {
    try {
      const merged = await this.fetchOwnBlobBeforePublish(store);
      if (this.destroyed) return;
      if (this.isIdenticalToLastPublished(merged)) {
        this.pendingStore = null;
        return;
      }
      const ciphertext = await nip44EncryptToSelf(JSON.stringify(merged));
      const createdAt = Math.max(
        Math.floor(Date.now() / 1_000),
        this.lastRemoteCreatedAt + 1,
      );
      const event = await signRelayEvent({
        kind: KIND_PROJECT_SIDEBAR_MEMBERSHIP,
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
        "Timed out publishing project sidebar membership.",
        "Failed to publish project sidebar membership.",
      );
      this.recordRemoteHead(event.created_at);
      this.lastPublishedStore = merged;
      this.pendingStore = null;
    } catch (error) {
      console.warn("[projectSidebarMembershipSync] publish failed:", error);
    }
  }

  async subscribe(
    onUpdate: (remote: RemoteProjectSidebarMembership) => void,
  ): Promise<() => Promise<void>> {
    return relayClient.subscribeLive(
      {
        kinds: [KIND_PROJECT_SIDEBAR_MEMBERSHIP],
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

  async bootstrap(localStore: ProjectSidebarMembershipStore) {
    const fetchResult = await this.fetchRemoteMembership();
    return runBootstrap({
      fetchResult,
      lastHead: this.lastRemoteCreatedAt,
      localStore,
      isLocalNonEmpty: (store) => Object.keys(store.projects).length > 0,
      publishFn: (store) => this.publishMembership(store),
    });
  }

  destroy(): void {
    this.destroyed = true;
    this.cancelPendingPublish();
    this.pendingStore = null;
  }
}
