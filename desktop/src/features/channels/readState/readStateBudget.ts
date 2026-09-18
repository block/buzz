/**
 * Byte-budget eviction for NIP-RS read-state blobs.
 *
 * Pure helpers, split out of `readStateManager` so the eviction policy lives
 * next to the recency signal it ranks by. Callers should prefer the manager's
 * `currentContexts()` / `splitContextsIntoSlots()`; these are exported for
 * direct unit testing.
 */
import {
  MSG_PREFIX,
  THREAD_PREFIX,
  readActionRecency,
} from "@/features/channels/readState/readStateFormat";

/**
 * Result of a `splitContextsIntoBudgetedSlots` call.
 */
export interface SlotSplitResult {
  /** Contexts record for each slot (primary slot first). */
  slots: Array<Record<string, number>>;
  /**
   * Extra slot IDs allocated beyond the first. Length is `slots.length - 1`.
   * The caller is responsible for persisting these.
   */
  extraSlotIds: string[];
}

/**
 * Partition `channelEntries` across slots so each slot's blob fits within
 * `maxBytes`. Thread/msg entries are added to the primary slot (index 0) and
 * trimmed to budget.
 *
 * `initialSlotCount` is the number of slots already available (≥ 1). If the
 * initial distribution doesn't fit, new slot IDs are generated via
 * `slotIdGenerator` until everything fits or `maxSlots` is reached.
 *
 * Returns `{ slots, extraSlotIds }` on success, or `null` when even `maxSlots`
 * slots can't accommodate all channel keys.
 *
 * Exported for unit testing; callers should prefer `splitContextsIntoSlots()`.
 */
export function splitContextsIntoBudgetedSlots(args: {
  channelEntries: [string, number][];
  threadMsgEntries: [string, number][];
  clientId: string;
  initialSlotCount: number;
  maxSlots: number;
  maxBytes: number;
  slotIdGenerator: () => string;
  contextSourceCreatedAt?: ReadonlyMap<string, number>;
}): SlotSplitResult | null {
  const {
    channelEntries,
    threadMsgEntries,
    clientId,
    initialSlotCount,
    maxSlots,
    maxBytes,
    slotIdGenerator,
    contextSourceCreatedAt,
  } = args;

  const encoder = new TextEncoder();
  const blobFor = (c: Record<string, number>) =>
    JSON.stringify({ v: 1, client_id: clientId, contexts: c });

  let slotCount = initialSlotCount;
  const extraSlotIds: string[] = [];

  // Distribute channel keys and check fit. Grow slot count until all fit.
  const distribute = (count: number): Array<Record<string, number>> => {
    const slotContexts: Array<Record<string, number>> = Array.from(
      { length: count },
      () => ({}),
    );
    for (let i = 0; i < channelEntries.length; i++) {
      const [key, ts] = channelEntries[i];
      slotContexts[i % count][key] = ts;
    }
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

  if (slotContexts.some((c) => encoder.encode(blobFor(c)).length > maxBytes)) {
    return null;
  }

  // Add thread/msg entries to the primary slot and trim to budget.
  for (const [key, ts] of threadMsgEntries) {
    slotContexts[0][key] = ts;
  }
  trimContextsToBudget(
    slotContexts[0],
    clientId,
    maxBytes,
    contextSourceCreatedAt,
  );

  return { slots: slotContexts, extraSlotIds };
}

/**
 * Result of a `trimContextsToBudget` call.
 */
export interface TrimResult {
  /** Number of entries removed from `contexts`. */
  evicted: number;
  /** True when the serialized blob fits within `maxBytes` after trimming. */
  fitsAfterTrim: boolean;
}

/**
 * Trim a contexts map to fit within `maxBytes` when serialized as the JSON
 * blob `{v:1, client_id, contexts}`. Evicts least-recently-read `msg:` entries
 * first, then least-recently-read `thread:` entries. Channel keys are never
 * evicted. Mutates `contexts` in place.
 *
 * Recency comes from `contextSourceCreatedAt` (see `readActionRecency`) and
 * falls back to the marker value. Ranking by the marker value alone evicts a
 * marker the user just created on an older message ahead of markers they last
 * touched days ago, so the read never survives the publish.
 *
 * Returns `{ evicted, fitsAfterTrim }`. `fitsAfterTrim` is false when the
 * remaining blob (channel keys only) still exceeds `maxBytes` — the caller
 * must not publish in that case.
 *
 * Exported for unit testing; callers should prefer `currentContexts()`.
 */
export function trimContextsToBudget(
  contexts: Record<string, number>,
  clientId: string,
  maxBytes: number,
  contextSourceCreatedAt?: ReadonlyMap<string, number>,
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
    if (key.startsWith(MSG_PREFIX)) {
      msgEntries.push([key, ts]);
    } else if (key.startsWith(THREAD_PREFIX)) {
      threadEntries.push([key, ts]);
    }
  }
  // Least-recently-read first within each tier.
  const byReadRecency = (a: [string, number], b: [string, number]) =>
    readActionRecency(a[0], a[1], contextSourceCreatedAt) -
    readActionRecency(b[0], b[1], contextSourceCreatedAt);
  msgEntries.sort(byReadRecency);
  threadEntries.sort(byReadRecency);

  // O(n) pass: subtract each entry's byte contribution from currentBytes and
  // collect entries to evict. The per-entry estimate is `,"key":timestamp`
  // (key.length + 3 bytes for `"`, `"`, `:` plus 1 comma) + timestamp digits.
  // This is an approximation — the final encode below is the authoritative check.
  const toEvict: string[] = [];
  for (const [key, ts] of [...msgEntries, ...threadEntries]) {
    if (currentBytes <= maxBytes) break;
    // Contribution: `,"key":timestamp` — comma + quoted key + colon + value
    currentBytes -= key.length + 3 + String(ts).length + 1;
    toEvict.push(key);
  }

  for (const key of toEvict) {
    delete contexts[key];
  }

  // Final authoritative check — handles JSON comma-accounting edge cases
  // (e.g. last-entry comma disappears) that the per-entry estimate ignores.
  const fitsAfterTrim = encoder.encode(blobFor(contexts)).length <= maxBytes;
  return { evicted: toEvict.length, fitsAfterTrim };
}
