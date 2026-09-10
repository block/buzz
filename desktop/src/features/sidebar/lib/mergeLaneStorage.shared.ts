// Shared generic implementation for merge-lane storage modules.
//
// Both channelStarsStorage.ts and channelMutesStorage.ts follow the same
// pattern: a per-(pubkey, relay) scoped key with one-time legacy migration,
// a claimLegacy ownership invariant, a bounded eviction, and per-window
// outbox helpers. Only the entry shape (starred vs muted), localStorage key
// prefix, capacity cap, and bound/parse functions differ.
//
// Each lane module imports these helpers and re-exports a typed surface.

import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";
import {
  clearOwnOutbox,
  enumerateOutbox,
  reclaimOutbox,
  writeOwnOutbox,
} from "./sidebarSyncWatermark";

export type OutboxWithMeta<S> = {
  store: S;
  preservedKey?: string;
};

// ─── Key helpers ──────────────────────────────────────────────────────────────

/**
 * Returns the localStorage key for a scoped (or legacy) entry.
 *
 * When `relayUrl` is provided the key is scoped to that relay so stores from
 * different communities/relays don't bleed across each other — in particular so
 * a non-empty store from relay A can never seed-publish onto a first-visited
 * relay B (Carl P1). When omitted the legacy pubkey-only key is returned.
 */
export function makeStorageKey(
  prefix: string,
  pubkey: string,
  relayUrl?: string,
): string {
  if (!relayUrl) return `${prefix}:${pubkey}`;
  const normalized = normalizeRelayUrl(relayUrl);
  return `${prefix}:${pubkey}:${encodeURIComponent(normalized)}`;
}

// ─── Legacy migration / ownership claim ──────────────────────────────────────

/**
 * Enforce the ownership invariant: expose `owned` only once the legacy key is
 * provably gone. Write the scoped copy when it is not already present, delete
 * the legacy key, and confirm the delete took. On failure return `defaultStore`,
 * and roll back a scoped copy WE just wrote only when the legacy key is provably
 * still present — if legacy is already gone (a delete that succeeded before a
 * later read threw), the scoped copy is the sole surviving copy and must be
 * kept, and if the probe itself throws we keep it too, favoring no-data-loss.
 * A kept-but-unproven copy is never exposed while an importable legacy key
 * remains (the read-time gate in readStore), so it can never seed early; the
 * next healthy read completes or retries the claim.
 */
export function claimLegacy<S>(
  key: string,
  legacyKey: string,
  owned: S,
  scopedExists: boolean,
  defaultStore: S,
): S {
  try {
    if (!scopedExists) window.localStorage.setItem(key, JSON.stringify(owned));
    window.localStorage.removeItem(legacyKey);
    if (window.localStorage.getItem(legacyKey) !== null) {
      // Delete did not take — do not expose data another relay could import.
      if (!scopedExists) window.localStorage.removeItem(key);
      return defaultStore;
    }
    return owned;
  } catch {
    if (!scopedExists) {
      try {
        // Roll back only if legacy is provably still importable: keeping the
        // scoped copy then would let a second relay scope claim legacy too
        // (double-seed). If legacy is already gone, the scoped copy is the only
        // one left and must be kept — rolling it back is permanent data loss.
        if (window.localStorage.getItem(legacyKey) !== null) {
          window.localStorage.removeItem(key);
        }
      } catch {
        // Residual (deliberate): the legacy delete AND this probe both throw,
        // so we cannot prove legacy gone. We keep the scoped copy to favor
        // no-data-loss; while legacy remains the read-time gate keeps it hidden
        // so it cannot seed early, but if storage partially recovers and this
        // window then switches relays, both scopes can carry the legacy value.
      }
    }
    return defaultStore;
  }
}

/**
 * Read the store for `pubkey` scoped to `relayUrl`.
 *
 * Enforces one invariant on every read: a scoped copy is exposed only while no
 * importable legacy (pubkey-only) key remains. If a non-empty legacy key still
 * exists we finish the claim inline via `claimLegacy`. On any storage failure
 * we return `defaultStore`, which cannot seed-publish (bootstrap gates on a
 * non-empty local store), leaving the claim to complete on a later read once
 * storage recovers.
 */
export function readStore<S>(
  storageKeyPrefix: string,
  pubkey: string,
  relayUrl: string | undefined,
  parseRaw: (raw: string | null) => S | null,
  defaultStore: S,
): S {
  try {
    const key = makeStorageKey(storageKeyPrefix, pubkey, relayUrl);
    const scoped = parseRaw(window.localStorage.getItem(key));

    // Unscoped read: the key IS the legacy key, so there is nothing to claim.
    if (!relayUrl) return scoped ?? defaultStore;

    const legacyKey = makeStorageKey(storageKeyPrefix, pubkey);
    const legacy = parseRaw(window.localStorage.getItem(legacyKey));
    const legacyHasData =
      legacy !== null &&
      Object.keys((legacy as { channels?: unknown }).channels ?? {}).length > 0;

    // No importable legacy value remains — an existing scoped copy is proven
    // owned. Short-circuits every read after a completed claim.
    if (!legacyHasData) return scoped ?? defaultStore;

    // A consumable legacy key still coexists with (or would seed) the scoped
    // key; neither is safe to expose until that legacy key is provably gone.
    return claimLegacy(
      key,
      legacyKey,
      scoped ?? legacy,
      scoped !== null,
      defaultStore,
    );
  } catch {
    return defaultStore;
  }
}

// ─── Outbox helpers ───────────────────────────────────────────────────────────

function legacyOutboxKey(
  outboxPrefix: string,
  pubkey: string,
  relayUrl: string,
): string {
  return `${outboxPrefix}:${pubkey}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`;
}

/**
 * Persist this window's unpublished edit under its own outbox key. Written
 * synchronously on every click as a single unconditional `setItem` (no
 * shared-key read-modify-write); resumed by merging every window's record on
 * next mount so a click made <2s before quit/community-switch is never dropped.
 */
export function writeOutbox<S>(
  outboxPrefix: string,
  pubkey: string,
  store: S,
  relayUrl: string,
  bound: (s: S) => S,
  preservedKey?: string,
): void {
  writeOwnOutbox(
    outboxPrefix,
    pubkey,
    relayUrl,
    bound(store),
    undefined,
    preservedKey,
  );
}

/**
 * Merge every window's persisted unpublished edit into one store for resume,
 * returning both the merged store and the most-recent explicit `preservedKey`
 * across ALL records (own and foreign), or null when no records exist.
 *
 * The preserved key is selected deterministically: the record with the highest
 * `queuedAt` that carries an explicit `preservedKey`; ties broken by key
 * string (max). This works whether the record was written by the current window
 * (isOwn) or by a prior window that has since closed — after a quit the nonce
 * is gone and the record is foreign, but its `preservedKey` is still durable
 * in localStorage and is recovered here (Kalvin P3 restart durability).
 *
 * Per-entry merge is order-independent so two windows' concurrent clicks on
 * different channels both survive. Both the fold and the bound apply the
 * selected preserved key so the clicked channel is never evicted before it
 * reaches the manager.
 */
export function readOutboxWithMeta<S>(
  outboxPrefix: string,
  pubkey: string,
  relayUrl: string,
  parse: (json: unknown) => S | null,
  merge: (a: S, b: S, preservedKey?: string) => S,
  defaultStore: S,
  bound: (s: S, preservedKey?: string) => S,
): OutboxWithMeta<S> | null {
  const records = enumerateOutbox(
    outboxPrefix,
    legacyOutboxKey(outboxPrefix, pubkey, relayUrl),
    pubkey,
    relayUrl,
    parse,
  );
  if (records.length === 0) return null;
  // Select the surviving preservedKey: max queuedAt among records that carry
  // an explicit preservedKey; ties broken by key string (max).
  let bestKey: string | undefined;
  let bestQueuedAt = -1;
  let bestStorageKey = "";
  for (const r of records) {
    if (r.preservedKey === undefined) continue;
    if (
      r.queuedAt > bestQueuedAt ||
      (r.queuedAt === bestQueuedAt && r.key > bestStorageKey)
    ) {
      bestKey = r.preservedKey;
      bestQueuedAt = r.queuedAt;
      bestStorageKey = r.key;
    }
  }
  // Fold all records and apply the capacity bound with the selected key so the
  // clicked channel is never evicted during the outbox read itself.
  // bestKey is threaded through every per-record merge so that no intermediate
  // merge — even across two durable records totaling 501 entries — evicts the
  // reserved click before the final defensive bound.
  const merged = records.reduce<S>(
    (acc, r) => merge(acc, r.store, bestKey),
    defaultStore,
  );
  return { store: bound(merged, bestKey), preservedKey: bestKey };
}

/** Clear this window's own outbox key (its edit published or is a no-op). */
export function clearOutbox(
  outboxPrefix: string,
  pubkey: string,
  relayUrl: string,
): void {
  clearOwnOutbox(outboxPrefix, pubkey, relayUrl);
}

/**
 * Reclaim foreign outbox keys the fetched relay head already subsumes: a
 * record is redundant when merging it into `head` yields `head` unchanged.
 * Never touches this window's own key. Call only after a successful head fetch.
 *
 * The `isSubsumedBy` probe receives each record's `preservedKey` so a
 * capacity-bounded proof can never evict the reserved channel and falsely
 * certify retention of a click the relay has not yet kept (Carl P3 / C4).
 */
export function reclaimSubsumedOutbox<S>(
  outboxPrefix: string,
  pubkey: string,
  relayUrl: string,
  parse: (json: unknown) => S | null,
  isSubsumedBy: (candidate: S, head: S, preservedKey?: string) => boolean,
  head: S,
): void {
  reclaimOutbox(
    outboxPrefix,
    legacyOutboxKey(outboxPrefix, pubkey, relayUrl),
    pubkey,
    relayUrl,
    parse,
    (record) => isSubsumedBy(record.store, head, record.preservedKey),
  );
}
