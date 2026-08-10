/**
 * Pure slot-splitting and trimming utilities for NIP-RS read-state publication.
 *
 * Extracted from readStateManager.ts to keep that file within the 1000-line
 * size ratchet. Both functions are stateless and depend only on the
 * readStateFormat constants and the TextEncoder API.
 */
import {
  isOverrideKey,
  unescapeFrontierKey,
  MSG_PREFIX,
  THREAD_PREFIX,
} from "@/features/channels/readState/readStateFormat";

/** Slot split result. */
export interface SlotSplitResult {
  slots: Array<Record<string, number>>;
  extraSlotIds: string[];
}

export interface TrimResult {
  evicted: number;
  fitsAfterTrim: boolean;
}

/** Partition channelEntries across slots; override groups pinned to slot 0. */
export function splitContextsIntoBudgetedSlots(args: {
  channelEntries: [string, number][];
  threadMsgEntries: [string, number][];
  clientId: string;
  initialSlotCount: number;
  maxSlots: number;
  maxBytes: number;
  slotIdGenerator: () => string;
}): SlotSplitResult | null {
  const {
    channelEntries,
    threadMsgEntries,
    clientId,
    initialSlotCount,
    maxSlots,
    maxBytes,
    slotIdGenerator,
  } = args;

  const encoder = new TextEncoder();
  const blobFor = (c: Record<string, number>) =>
    JSON.stringify({ v: 1, client_id: clientId, contexts: c });

  const overrideRawIds = new Set<string>();
  for (const [key] of channelEntries) {
    if (isOverrideKey(key)) overrideRawIds.add(key.slice(5));
  }
  const pinnedEntries: [string, number][] = [];
  const roundRobinEntries: [string, number][] = [];
  for (const [key, ts] of channelEntries) {
    if (isOverrideKey(key) || overrideRawIds.has(unescapeFrontierKey(key))) {
      pinnedEntries.push([key, ts]);
    } else {
      roundRobinEntries.push([key, ts]);
    }
  }
  let slotCount = initialSlotCount;
  const extraSlotIds: string[] = [];
  const distribute = (count: number): Array<Record<string, number>> => {
    const slotContexts: Array<Record<string, number>> = Array.from(
      { length: count },
      () => ({}),
    );
    for (let i = 0; i < roundRobinEntries.length; i++) {
      const [key, ts] = roundRobinEntries[i];
      slotContexts[i % count][key] = ts;
    }
    for (const [key, ts] of pinnedEntries) slotContexts[0][key] = ts;
    return slotContexts;
  };

  let slotContexts = distribute(slotCount);
  while (
    slotContexts.some((c) => encoder.encode(blobFor(c)).length > maxBytes) &&
    slotCount < maxSlots
  ) {
    extraSlotIds.push(slotIdGenerator());
    slotCount++;
    slotContexts = distribute(slotCount);
  }
  if (slotContexts.some((c) => encoder.encode(blobFor(c)).length > maxBytes))
    return null;
  for (const [key, ts] of threadMsgEntries) slotContexts[0][key] = ts;
  trimContextsToBudget(slotContexts[0], clientId, maxBytes);
  return { slots: slotContexts, extraSlotIds };
}

/** Trim a contexts map to fit within `maxBytes`. Evicts oldest msg:/thread: entries; channel/ov_* keys never evicted. */
export function trimContextsToBudget(
  contexts: Record<string, number>,
  clientId: string,
  maxBytes: number,
): TrimResult {
  const encoder = new TextEncoder();
  const blobFor = (c: Record<string, number>) =>
    JSON.stringify({ v: 1, client_id: clientId, contexts: c });

  let currentBytes = encoder.encode(blobFor(contexts)).length;
  if (currentBytes <= maxBytes) {
    return { evicted: 0, fitsAfterTrim: true };
  }

  const msgEntries: [string, number][] = [];
  const threadEntries: [string, number][] = [];
  for (const [key, ts] of Object.entries(contexts)) {
    if (isOverrideKey(key)) continue;
    if (key.startsWith(MSG_PREFIX)) msgEntries.push([key, ts]);
    else if (key.startsWith(THREAD_PREFIX)) threadEntries.push([key, ts]);
  }
  msgEntries.sort((a, b) => a[1] - b[1]);
  threadEntries.sort((a, b) => a[1] - b[1]);
  const toEvict: string[] = [];
  for (const [key, ts] of [...msgEntries, ...threadEntries]) {
    if (currentBytes <= maxBytes) break;
    currentBytes -= key.length + 3 + String(ts).length + 1;
    toEvict.push(key);
  }
  for (const key of toEvict) delete contexts[key];
  const fitsAfterTrim = encoder.encode(blobFor(contexts)).length <= maxBytes;
  return { evicted: toEvict.length, fitsAfterTrim };
}
