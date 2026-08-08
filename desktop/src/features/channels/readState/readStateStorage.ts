import {
  isPlainRecord,
  localIsoToUnixSeconds,
  localPublishableContextKey,
  localReadStateKey,
  localSourceCreatedAtKey,
  LOCAL_MAX_PRUNABLE_CONTEXTS,
  MSG_PREFIX,
  READ_STATE_HORIZON_SECONDS,
  THREAD_PREFIX,
  type OverrideRegister,
} from "@/features/channels/readState/readStateFormat";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";
import type {
  PendingIntent,
  PendingIntentOp,
} from "@/features/channels/pendingOverrideIntents";
import type {
  ForcedUnreadEntry,
  ForcedUnreadSource,
} from "@/features/channels/forcedUnreadStore";

/**
 * Maximum valid protocol timestamp (unix seconds): uint32 ceiling (year 2106).
 * Matches the limit used for override counters so a stored readTarget that
 * overflows uint32 is silently rejected before it can reach Date.toISOString().
 */
const MAX_PROTOCOL_TIMESTAMP = 0xffffffff;

export type StoredReadState = {
  contexts: Map<string, number>;
  publishableContextIds: Set<string>;
  contextSourceCreatedAt: Map<string, number>;
  overrideRegisters: Map<string, StoredOverrideEntry>;
  appliedReceipts: Map<string, AppliedReceipt>;
  /** Pending intents from the v2 blob (consolidated from pending-intents.v1). */
  pendingIntents: Map<string, PendingIntent>;
  /** Next generation counter for the pending intent queue. */
  nextGen: number;
};

/**
 * Applied receipt — co-committed with the register mutation in
 * `persistLocalState()` (Amendment C).
 *
 * A surviving intent whose `{intentGen, op}` matches a receipt is
 * *already applied*: hydration completes cleanup + compare-delete WITHOUT
 * another S/C bump.
 *
 * Lifetime: receipt lifetime ⊆ intent lifetime.  The receipt is deleted in
 * the same store transaction as its intent (both are in the same v2 blob).
 * Receipts cannot accumulate.
 */
export type AppliedReceipt = {
  intentGen: number;
  op: PendingIntentOp;
};

function mergeLocalStorageKey(
  contexts: Map<string, number>,
  key: string,
): void {
  const raw = localStorage.getItem(key);
  if (!raw) return;

  try {
    const parsed = JSON.parse(raw);
    if (!isPlainRecord(parsed)) return;

    for (const [channelId, value] of Object.entries(parsed)) {
      const unixSeconds = localIsoToUnixSeconds(value);
      if (unixSeconds === null) continue;
      const current = contexts.get(channelId) ?? 0;
      if (unixSeconds > current) {
        contexts.set(channelId, unixSeconds);
      }
    }
  } catch (error) {
    console.debug("[ReadStateManager] storage: contexts JSON corrupt:", error);
    // Corrupt localStorage, ignore.
  }
}

function readPublishableContextIds(pubkey: string): Set<string> {
  const result = new Set<string>();
  const raw = localStorage.getItem(localPublishableContextKey(pubkey));
  if (!raw) return result;

  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return result;

    for (const value of parsed) {
      if (typeof value === "string") {
        result.add(value);
      }
    }
  } catch (error) {
    console.debug(
      "[ReadStateManager] storage: publishableContextIds JSON corrupt:",
      error,
    );
    // Corrupt localStorage, ignore.
  }

  return result;
}

function readContextSourceCreatedAt(pubkey: string): Map<string, number> {
  const result = new Map<string, number>();
  const raw = localStorage.getItem(localSourceCreatedAtKey(pubkey));
  if (!raw) return result;

  try {
    const parsed = JSON.parse(raw);
    if (!isPlainRecord(parsed)) return result;

    for (const [key, value] of Object.entries(parsed)) {
      if (typeof value === "number" && Number.isInteger(value) && value >= 0) {
        result.set(key, value);
      }
    }
  } catch (error) {
    console.debug(
      "[ReadStateManager] storage: sourceCreatedAt JSON corrupt:",
      error,
    );
    // Corrupt localStorage, ignore.
  }

  return result;
}

export function readStoredReadState(pubkey: string): StoredReadState {
  const contexts = new Map<string, number>();
  mergeLocalStorageKey(contexts, localReadStateKey(pubkey));

  const overrideState = readOverrideState(pubkey);
  return {
    contexts,
    publishableContextIds: readPublishableContextIds(pubkey),
    contextSourceCreatedAt: readContextSourceCreatedAt(pubkey),
    overrideRegisters: overrideState.registers,
    appliedReceipts: overrideState.receipts,
    pendingIntents: overrideState.pendingIntents,
    nextGen: overrideState.nextGen,
  };
}

function isPrunableContextKey(contextId: string): boolean {
  return (
    contextId.startsWith(MSG_PREFIX) || contextId.startsWith(THREAD_PREFIX)
  );
}

// Key for persisted override state: registers + their frontier timestamps in one atomic write.
// v2 stores {s, c, b, f} per context where f = the last known effective frontier.
function localOverrideStateKey(pubkey: string): string {
  return `buzz.nip-rs.override-state.v2:${pubkey}`;
}

// Legacy key retained for migration reads only — never written to after v2 migration.
function localOverrideRegistersKeyLegacy(pubkey: string): string {
  return `buzz.nip-rs.override-registers.v1:${pubkey}`;
}

export type StoredOverrideEntry = OverrideRegister & { f: number };

type OverrideStateBlob = {
  registers: Map<string, StoredOverrideEntry>;
  receipts: Map<string, AppliedReceipt>;
  pendingIntents: Map<string, PendingIntent>;
  nextGen: number;
};

/**
 * Parse and validate a raw JSON value as a `ForcedUnreadEntry`.
 *
 * Returns `{ ok: true, value: ForcedUnreadEntry }` on success or
 * `{ ok: false }` if the value is present but malformed.
 * Returns `{ ok: true, value: undefined }` if the raw value is `undefined`
 * (absent field — no prior entry stored).
 *
 * `null` is a valid legacy v1 null marker and is returned as
 * `{ ok: true, value: null }` — NOT as a failure sentinel.
 *
 * Valid shapes (matching the `ForcedUnreadEntry` union):
 *  - `number`                                        — legacy v1 scalar marker
 *  - `null`                                          — legacy v1 null marker
 *  - `{ markerAtWhenForced: number | null, sources: ForcedUnreadSource[] }`
 *
 * Structured entries are accepted only when ALL members of `sources` are
 * valid ForcedUnreadSource values — no silent filtering of partial arrays.
 */
function parseForcedUnreadEntry(
  raw: unknown,
): { ok: true; value: ForcedUnreadEntry | undefined } | { ok: false } {
  if (raw === undefined) return { ok: true, value: undefined };
  if (raw === null) return { ok: true, value: null };
  if (typeof raw === "number") {
    if (Number.isFinite(raw) && raw >= 0 && raw <= MAX_PROTOCOL_TIMESTAMP)
      return { ok: true, value: raw };
    return { ok: false }; // invalid numeric marker
  }
  if (typeof raw === "object" && !Array.isArray(raw)) {
    const obj = raw as Record<string, unknown>;
    const marker = obj.markerAtWhenForced;
    if (
      marker !== null &&
      (typeof marker !== "number" ||
        !Number.isFinite(marker) ||
        marker < 0 ||
        marker > MAX_PROTOCOL_TIMESTAMP)
    )
      return { ok: false };
    const sources = obj.sources;
    if (!Array.isArray(sources)) return { ok: false };
    // All members must be valid — no silent partial filtering.
    for (const s of sources) {
      if (s !== "inbox" && s !== "manual") return { ok: false };
    }
    if (sources.length === 0) return { ok: false };
    return {
      ok: true,
      value: {
        markerAtWhenForced: (marker as number | null) ?? null,
        sources: sources as ForcedUnreadSource[],
      },
    };
  }
  return { ok: false }; // unrecognised shape
}

function readOverrideState(pubkey: string): OverrideStateBlob {
  const registers = new Map<string, StoredOverrideEntry>();
  const receipts = new Map<string, AppliedReceipt>();
  const pendingIntents = new Map<string, PendingIntent>();
  let nextGen = 1;
  const raw = localStorage.getItem(localOverrideStateKey(pubkey));
  if (raw) {
    try {
      const parsed = JSON.parse(raw);
      if (isPlainRecord(parsed)) {
        // Registers are stored under the "r" key (or top-level for compat).
        const registersObj = isPlainRecord(parsed.r) ? parsed.r : parsed;
        for (const [rawCtx, value] of Object.entries(registersObj)) {
          // Skip reserved top-level keys.
          if (rawCtx === "receipts" || rawCtx === "pi" || rawCtx === "ng")
            continue;
          if (!isPlainRecord(value)) continue;
          const { s, c, b, f } = value;
          if (
            typeof s === "number" &&
            Number.isInteger(s) &&
            s >= 0 &&
            s <= 0xffffffff &&
            typeof c === "number" &&
            Number.isInteger(c) &&
            c >= 0 &&
            c <= 0xffffffff &&
            typeof b === "number" &&
            Number.isInteger(b) &&
            b >= 0 &&
            typeof f === "number" &&
            Number.isInteger(f) &&
            f >= 0
          ) {
            registers.set(rawCtx, { s, c, b, f });
          }
        }
        // Parse receipts from the "receipts" sub-object.
        if (isPlainRecord(parsed.receipts)) {
          for (const [channelId, receipt] of Object.entries(parsed.receipts)) {
            if (!isPlainRecord(receipt)) continue;
            const { intentGen, op } = receipt as {
              intentGen?: unknown;
              op?: unknown;
            };
            if (
              typeof intentGen !== "number" ||
              !Number.isSafeInteger(intentGen) ||
              intentGen < 1
            )
              continue;
            if (op !== "unread" && op !== "read") continue;
            receipts.set(channelId, { intentGen, op });
          }
        }
        // Parse pending intents from the "pi" sub-object.
        if (isPlainRecord(parsed.pi)) {
          for (const [channelId, entry] of Object.entries(parsed.pi)) {
            if (!isPlainRecord(entry)) continue;
            const { gen, op, sourceScope, readTarget, priorForcedEntry } =
              entry as {
                gen?: unknown;
                op?: unknown;
                sourceScope?: unknown;
                readTarget?: unknown;
                priorForcedEntry?: unknown;
              };
            if (
              typeof gen !== "number" ||
              !Number.isSafeInteger(gen) ||
              gen < 1
            )
              continue;
            if (op !== "unread" && op !== "read") continue;
            // sourceScope must be a valid ForcedUnreadSource or absent.
            if (
              sourceScope !== undefined &&
              sourceScope !== "inbox" &&
              sourceScope !== "manual"
            )
              continue;
            // readTarget must be a finite integer in protocol timestamp range.
            if (readTarget !== undefined) {
              if (
                typeof readTarget !== "number" ||
                !Number.isFinite(readTarget) ||
                !Number.isInteger(readTarget) ||
                readTarget < 0 ||
                readTarget > MAX_PROTOCOL_TIMESTAMP
              )
                continue;
            }
            // priorForcedEntry must be a valid ForcedUnreadEntry or absent.
            const parsedPrior = parseForcedUnreadEntry(priorForcedEntry);
            if (!parsedPrior.ok) continue;
            const intent: PendingIntent = {
              gen,
              op,
              ...(sourceScope !== undefined
                ? { sourceScope: sourceScope as string }
                : {}),
              ...(readTarget !== undefined ? { readTarget } : {}),
              ...(parsedPrior.value !== undefined
                ? { priorForcedEntry: parsedPrior.value }
                : {}),
            };
            pendingIntents.set(channelId, intent);
          }
        }
        // Parse nextGen.
        if (
          typeof parsed.ng === "number" &&
          Number.isSafeInteger(parsed.ng) &&
          parsed.ng >= 1
        ) {
          nextGen = parsed.ng;
        }
        // Normalize nextGen to be strictly greater than all valid intent and
        // receipt generations so a corrupt or reused `ng` cannot cause a new
        // enqueue to receive a generation that matches an existing receipt,
        // which would silently discard the fresh action via the Amendment C
        // already-applied check.
        //
        // If the increment would be unrepresentable (> Number.MAX_SAFE_INTEGER),
        // fall back to a fresh safe value — any safe value above existing gens
        // is acceptable; we prefer the smallest representable one.
        {
          let maxObservedGen = 0;
          for (const intent of pendingIntents.values()) {
            if (intent.gen > maxObservedGen) maxObservedGen = intent.gen;
          }
          for (const receipt of receipts.values()) {
            if (receipt.intentGen > maxObservedGen)
              maxObservedGen = receipt.intentGen;
          }
          const requiredNextGen = maxObservedGen + 1;
          if (
            Number.isSafeInteger(requiredNextGen) &&
            requiredNextGen > nextGen
          ) {
            nextGen = requiredNextGen;
          } else if (!Number.isSafeInteger(requiredNextGen)) {
            // maxObservedGen is already at or near MAX_SAFE_INTEGER — reset to
            // a fresh safe value.  Existing gens become unreachable for amendment-C
            // matching (intentionally: the corrupt blob is untrustworthy).
            nextGen = 1;
          }
        }
      }
    } catch {
      // Corrupt storage — return empty; repopulated on next ingest.
    }
    return { registers, receipts, pendingIntents, nextGen };
  }
  // Migration: read from legacy v1 key if v2 key absent.
  const legacyRaw = localStorage.getItem(
    localOverrideRegistersKeyLegacy(pubkey),
  );
  if (!legacyRaw) return { registers, receipts, pendingIntents, nextGen };
  try {
    const parsed = JSON.parse(legacyRaw);
    if (!isPlainRecord(parsed))
      return { registers, receipts, pendingIntents, nextGen };
    for (const [rawCtx, value] of Object.entries(parsed)) {
      if (!isPlainRecord(value)) continue;
      const { s, c, b } = value;
      if (
        typeof s === "number" &&
        Number.isInteger(s) &&
        s >= 0 &&
        s <= 0xffffffff &&
        typeof c === "number" &&
        Number.isInteger(c) &&
        c >= 0 &&
        c <= 0xffffffff &&
        typeof b === "number" &&
        Number.isInteger(b) &&
        b >= 0
      ) {
        registers.set(rawCtx, { s, c, b, f: 0 }); // frontier unknown from legacy key
      }
    }
  } catch {
    // Corrupt legacy storage — ignore.
  }
  return { registers, receipts, pendingIntents, nextGen };
}

/**
 * Drops msg:/thread: markers older than the relay's 7-day horizon, then caps
 * the survivors at LOCAL_MAX_PRUNABLE_CONTEXTS (oldest first). Channel keys
 * are never pruned — they are small, bounded by membership, and losing one
 * would resurrect the channel's unread badge. Mirrors the eviction order the
 * publish path already applies in trimContextsToBudget.
 */
export function pruneStaleContexts(
  contexts: ReadonlyMap<string, number>,
  nowUnixSeconds: number,
): Map<string, number> {
  const cutoff = nowUnixSeconds - READ_STATE_HORIZON_SECONDS;
  const kept = new Map<string, number>();
  const prunable: [string, number][] = [];

  for (const [contextId, timestamp] of contexts) {
    if (!isPrunableContextKey(contextId)) {
      kept.set(contextId, timestamp);
    } else if (timestamp >= cutoff) {
      prunable.push([contextId, timestamp]);
    }
  }

  if (prunable.length > LOCAL_MAX_PRUNABLE_CONTEXTS) {
    prunable.sort((a, b) => b[1] - a[1]);
    prunable.length = LOCAL_MAX_PRUNABLE_CONTEXTS;
  }
  for (const [contextId, timestamp] of prunable) {
    kept.set(contextId, timestamp);
  }
  return kept;
}

export function writeStoredReadState(
  pubkey: string,
  contexts: ReadonlyMap<string, number>,
  publishableContextIds: ReadonlySet<string>,
  contextSourceCreatedAt: ReadonlyMap<string, number>,
  overrideRegisters: ReadonlyMap<string, StoredOverrideEntry>,
  appliedReceipts: ReadonlyMap<string, AppliedReceipt>,
  pendingIntents: ReadonlyMap<string, PendingIntent>,
  nextGen: number,
): boolean {
  const pruned = pruneStaleContexts(contexts, Math.floor(Date.now() / 1_000));

  const state: Record<string, string> = {};
  for (const [contextId, timestamp] of pruned) {
    state[contextId] = new Date(timestamp * 1_000).toISOString();
  }

  // Persist override registers, applied receipts, and pending intents atomically
  // in one JSON blob — a single write ensures they are never torn.
  //
  // Format: {
  //   r:        { <channelId>: {s,c,b,f} },
  //   receipts: { <channelId>: {intentGen, op} },  // omitted when empty
  //   pi:       { <channelId>: {gen, op, sourceScope?, readTarget?} },  // omitted when empty
  //   ng:       <number>,  // next generation counter; omitted when 1
  // }
  //
  // This is the ACTION + INTENT COMMIT POINT: if ok4 is true, the action,
  // receipt, and pending intent state are all durably committed together.
  // If ok4 is false, the caller must roll back ALL state.
  const registersObj: Record<
    string,
    { s: number; c: number; b: number; f: number }
  > = {};
  for (const [rawCtx, entry] of overrideRegisters) {
    registersObj[rawCtx] = { s: entry.s, c: entry.c, b: entry.b, f: entry.f };
  }
  const receiptsObj: Record<
    string,
    { intentGen: number; op: PendingIntentOp }
  > = {};
  for (const [channelId, receipt] of appliedReceipts) {
    receiptsObj[channelId] = { intentGen: receipt.intentGen, op: receipt.op };
  }
  const piObj: Record<
    string,
    {
      gen: number;
      op: PendingIntentOp;
      sourceScope?: string;
      readTarget?: number;
      priorForcedEntry?: ForcedUnreadEntry;
    }
  > = {};
  for (const [channelId, intent] of pendingIntents) {
    const entry: {
      gen: number;
      op: PendingIntentOp;
      sourceScope?: string;
      readTarget?: number;
      priorForcedEntry?: ForcedUnreadEntry;
    } = {
      gen: intent.gen,
      op: intent.op,
    };
    if (intent.sourceScope !== undefined)
      entry.sourceScope = intent.sourceScope;
    if (intent.readTarget !== undefined) entry.readTarget = intent.readTarget;
    if (intent.priorForcedEntry !== undefined)
      entry.priorForcedEntry = intent.priorForcedEntry;
    piObj[channelId] = entry;
  }
  const ok4 = setLocalStorageItemWithRecovery(
    localOverrideStateKey(pubkey),
    JSON.stringify({
      r: registersObj,
      ...(Object.keys(receiptsObj).length > 0 ? { receipts: receiptsObj } : {}),
      ...(Object.keys(piObj).length > 0 ? { pi: piObj } : {}),
      ...(nextGen > 1 ? { ng: nextGen } : {}),
    }),
  );

  // If the commit-point write failed, return immediately — do NOT write any
  // ancillary keys.  The caller will roll back all in-memory and slot state.
  // "v2 false ⇒ nothing durable anywhere" is the contract.
  if (!ok4) return false;

  // Ancillary writes: frontier cache, publishable set, source timestamps.
  // Best-effort — failures do not fail the action.
  setLocalStorageItemWithRecovery(
    localReadStateKey(pubkey),
    JSON.stringify(state),
  );
  setLocalStorageItemWithRecovery(
    localPublishableContextKey(pubkey),
    JSON.stringify([...publishableContextIds].filter((id) => pruned.has(id))),
  );

  const sourceState: Record<string, number> = {};
  for (const [contextId, createdAt] of contextSourceCreatedAt) {
    if (pruned.has(contextId)) {
      sourceState[contextId] = createdAt;
    }
  }
  setLocalStorageItemWithRecovery(
    localSourceCreatedAtKey(pubkey),
    JSON.stringify(sourceState),
  );

  // The commit point: only the v2 override write determines action success.
  return ok4;
}
