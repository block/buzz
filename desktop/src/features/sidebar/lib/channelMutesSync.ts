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
  readChannelMutesStore,
  type ChannelMuteStore,
} from "./channelMutesStorage";

const D_TAG = "channel-mutes";
const DEBOUNCE_MS = 2_000;

export type RemoteMutes = {
  store: ChannelMuteStore;
  createdAt: number;
  eventId: string;
};

let debounceTimer: number | null = null;
let lastRemoteCreatedAt = 0;
let pendingStore: ChannelMuteStore | null = null;

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

export async function fetchRemoteMutes(
  pubkey: string,
): Promise<RemoteMutes | null> {
  try {
    const events = await relayClient.fetchEvents({
      kinds: [KIND_CHANNEL_MUTES],
      authors: [pubkey],
      "#d": [D_TAG],
      limit: 1,
    });
    if (events.length === 0) return null;
    if (events[0].pubkey !== pubkey) return null;
    const result = await decryptAndParse(events[0]);
    if (result) {
      lastRemoteCreatedAt = Math.max(lastRemoteCreatedAt, result.createdAt);
      const local = readChannelMutesStore(pubkey);
      return { ...result, store: mergeStores(local, result.store) };
    }
    return result;
  } catch {
    return null;
  }
}

export function cancelPendingMutePublish(): void {
  if (debounceTimer !== null) {
    window.clearTimeout(debounceTimer);
    debounceTimer = null;
  }
}

export function getPendingMuteStore(): ChannelMuteStore | null {
  return pendingStore;
}

export function publishMutes(store: ChannelMuteStore): void {
  pendingStore = store;
  if (debounceTimer !== null) {
    window.clearTimeout(debounceTimer);
  }
  debounceTimer = window.setTimeout(() => {
    debounceTimer = null;
    void doPublish(store);
  }, DEBOUNCE_MS);
}

async function doPublish(store: ChannelMuteStore): Promise<void> {
  try {
    const payload = {
      version: 1,
      channels: store.channels,
    };
    const ciphertext = await nip44EncryptToSelf(JSON.stringify(payload));
    const createdAt = Math.max(
      Math.floor(Date.now() / 1_000),
      lastRemoteCreatedAt + 1,
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
    await relayClient.publishEvent(
      event,
      "Timed out publishing channel mutes.",
      "Failed to publish channel mutes.",
    );
    lastRemoteCreatedAt = Math.max(lastRemoteCreatedAt, event.created_at);
    pendingStore = null;
  } catch (error) {
    console.warn("[channelMutesSync] publish failed:", error);
  }
}

export async function subscribeToMutes(
  pubkey: string,
  onUpdate: (remote: RemoteMutes) => void,
): Promise<() => Promise<void>> {
  return relayClient.subscribeLive(
    {
      kinds: [KIND_CHANNEL_MUTES],
      authors: [pubkey],
      "#d": [D_TAG],
      limit: 0,
    },
    (event: RelayEvent) => {
      if (event.pubkey !== pubkey) return;
      void decryptAndParse(event).then((result) => {
        if (result) {
          lastRemoteCreatedAt = Math.max(lastRemoteCreatedAt, result.createdAt);
          onUpdate(result);
        }
      });
    },
  );
}

export function resetMuteSyncState(): void {
  if (debounceTimer !== null) {
    window.clearTimeout(debounceTimer);
    debounceTimer = null;
  }
  lastRemoteCreatedAt = 0;
  pendingStore = null;
}
