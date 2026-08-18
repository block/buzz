import { nip44DecryptFromSelf } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import {
  isValidBlob,
  isValidReadStateDTag,
  sanitizeContexts,
  type ReadStateBlob,
} from "@/features/channels/readState/readStateFormat";

export type ReadStateDecrypt = (ciphertext: string) => Promise<string>;

export type ParsedReadStateEvent = {
  dTag: string;
  blob: ReadStateBlob;
  createdAt: number;
};

export async function parseReadStateEvent(
  event: RelayEvent,
  pubkey: string,
  decrypt: ReadStateDecrypt = nip44DecryptFromSelf,
): Promise<ParsedReadStateEvent | null> {
  if (event.pubkey !== pubkey) return null;

  const dTags = event.tags.filter((tag) => tag[0] === "d");
  if (dTags.length !== 1) return null;
  const dTag = dTags[0]?.[1];
  if (!isValidReadStateDTag(dTag)) return null;

  const tTags = event.tags.filter(
    (tag) => tag[0] === "t" && tag[1] === "read-state",
  );
  if (tTags.length !== 1) return null;

  try {
    const plaintext = await decrypt(event.content);
    const parsed = JSON.parse(plaintext);
    if (!isValidBlob(parsed)) return null;
    return {
      dTag,
      blob: {
        v: 1,
        client_id: parsed.client_id,
        contexts: sanitizeContexts(parsed.contexts),
      },
      createdAt: event.created_at,
    };
  } catch (error) {
    console.debug(
      `[ReadStateSnapshot] decrypt/parse failed event=${event.id.substring(0, 8)}…:`,
      error,
    );
    return null;
  }
}

export async function mergeReadStateEvents(
  events: RelayEvent[],
  pubkey: string,
  decrypt?: ReadStateDecrypt,
): Promise<Map<string, number>> {
  const contexts = new Map<string, number>();

  for (const event of events) {
    const parsed = await parseReadStateEvent(event, pubkey, decrypt);
    if (!parsed) continue;

    for (const [contextId, timestamp] of Object.entries(parsed.blob.contexts)) {
      const current = contexts.get(contextId) ?? 0;
      if (timestamp > current) {
        contexts.set(contextId, timestamp);
      }
    }
  }

  return contexts;
}

export function getSnapshotReadTimestamp(
  contexts: ReadonlyMap<string, number>,
  contextId: string,
): number | null {
  return contexts.get(contextId) ?? null;
}
