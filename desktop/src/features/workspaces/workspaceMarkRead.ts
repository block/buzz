import { READ_STATE_MAX_PLAINTEXT_BYTES } from "@/features/channels/readState/readStateFormat";
import type { Workspace } from "@/features/workspaces/types";
import { fetchObservedChannels } from "@/features/workspaces/workspaceUnreadObserver";
import { withReadOnlyRelayClient } from "@/shared/api/readOnlyRelayClient";
import type { RelaySubscriptionFilter } from "@/shared/api/relayClientShared";
import { nip44EncryptToSelf, signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_READ_STATE } from "@/shared/constants/kinds";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";

// Slot ceiling mirrors READ_STATE_MAX_SLOTS; beyond it we drop the remainder
// rather than flood the relay — the next regular client publish catches up.
const OBSERVER_MAX_SLOTS = 8;

const OBSERVER_CLIENT_ID_KEY_PREFIX = "buzz.nip-rs.observer-client-id";
const OBSERVER_SLOT_ID_KEY_PREFIX = "buzz.nip-rs.observer-slot-id";

type MarkReadRelay = {
  fetchEvents(filter: RelaySubscriptionFilter): Promise<RelayEvent[]>;
  publishEvent(event: RelayEvent): Promise<void>;
};

type SignEvent = (input: {
  kind: number;
  content: string;
  createdAt?: number;
  tags: string[][];
}) => Promise<RelayEvent>;

/**
 * Split channel read markers into NIP-RS blobs that each fit the single-slot
 * plaintext budget. Greedy fill, order-preserving. Returns at most `maxSlots`
 * chunks — overflow channels are dropped (grow-only semantics make this safe;
 * they simply stay unread until read normally).
 */
export function chunkChannelContexts(
  channelIds: string[],
  readAt: number,
  clientId: string,
  maxBytes: number = READ_STATE_MAX_PLAINTEXT_BYTES,
  maxSlots: number = OBSERVER_MAX_SLOTS,
): Array<Record<string, number>> {
  const encoder = new TextEncoder();
  const blobBytes = (contexts: Record<string, number>) =>
    encoder.encode(JSON.stringify({ v: 1, client_id: clientId, contexts }))
      .length;

  const chunks: Array<Record<string, number>> = [];
  let current: Record<string, number> = {};

  for (const channelId of channelIds) {
    const candidate = { ...current, [channelId]: readAt };
    if (Object.keys(current).length > 0 && blobBytes(candidate) > maxBytes) {
      chunks.push(current);
      if (chunks.length >= maxSlots) {
        return chunks;
      }
      current = { [channelId]: readAt };
      continue;
    }
    current = candidate;
  }

  if (Object.keys(current).length > 0) {
    chunks.push(current);
  }
  return chunks;
}

/**
 * Publish read-state blobs marking every observed channel on an INACTIVE
 * workspace's relay as read-now. Grow-only NIP-RS semantics: other devices
 * max-merge these markers, so publishing "everything read at `nowSeconds`"
 * can never regress a marker that is already further ahead.
 */
export async function publishWorkspaceReadState(args: {
  client: MarkReadRelay;
  pubkey: string;
  relayUrl: string;
  nowSeconds?: number;
  encrypt?: (plaintext: string) => Promise<string>;
  sign?: SignEvent;
}): Promise<void> {
  const { client, pubkey, relayUrl } = args;
  const encrypt = args.encrypt ?? nip44EncryptToSelf;
  const sign = args.sign ?? signRelayEvent;
  const nowSeconds = args.nowSeconds ?? Math.floor(Date.now() / 1_000);

  const channels = await fetchObservedChannels(client, pubkey);
  if (channels.length === 0) return;

  const clientId = persistedId(`${OBSERVER_CLIENT_ID_KEY_PREFIX}:${pubkey}`);
  const chunks = chunkChannelContexts(
    channels.map((channel) => channel.id),
    nowSeconds,
    clientId,
  );

  for (let index = 0; index < chunks.length; index++) {
    const slotId = persistedId(
      `${OBSERVER_SLOT_ID_KEY_PREFIX}:${pubkey}:${relayUrl}:${index}`,
    );
    const ciphertext = await encrypt(
      JSON.stringify({ v: 1, client_id: clientId, contexts: chunks[index] }),
    );
    const event = await sign({
      kind: KIND_READ_STATE,
      content: ciphertext,
      createdAt: nowSeconds,
      tags: [
        ["d", `read-state:${slotId}`],
        ["t", "read-state"],
      ],
    });
    await client.publishEvent(event);
  }
}

export async function markWorkspaceRead(
  workspace: Workspace,
  pubkey: string,
): Promise<void> {
  await withReadOnlyRelayClient(workspace.relayUrl, (client) =>
    publishWorkspaceReadState({
      client,
      pubkey,
      relayUrl: workspace.relayUrl,
    }),
  );
}

function persistedId(key: string): string {
  let value = localStorage.getItem(key);
  if (!value) {
    const bytes = new Uint8Array(16);
    crypto.getRandomValues(bytes);
    value = Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
    setLocalStorageItemWithRecovery(key, value);
  }
  return value;
}
