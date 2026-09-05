import {
  clearOutbox,
  makeStorageKey,
  readOutboxWithMeta,
  readStore,
  reclaimSubsumedOutbox,
  writeOutbox,
  type OutboxWithMeta,
} from "./mergeLaneStorage.shared";

const STORAGE_KEY_PREFIX = "buzz-channel-mutes.v1";
const OUTBOX_KEY_PREFIX = "buzz-channel-mutes-outbox.v1";
export const MAX_CHANNEL_MUTE_ENTRIES = 500;

export type ChannelMuteEntry = {
  muted: boolean;
  updatedAt: number;
  // Per-channel Lamport revision. Breaks a same-second `updatedAt` tie that the
  // integer clock cannot resolve. Absent in blobs from an older build ⇒ read as
  // 0 (a valid, mergeable value), so the payload stays `version: 1` and older
  // builds still parse our blobs.
  rev: number;
};

export type ChannelMuteStore = {
  version: 1;
  channels: Record<string, ChannelMuteEntry>;
};

export const DEFAULT_STORE: ChannelMuteStore = Object.freeze({
  version: 1,
  channels: {},
});

/**
 * Returns the localStorage key for channel mutes.
 *
 * When `relayUrl` is provided the key is scoped to that relay (normalized via
 * the same `normalizeRelayUrl` used by all relay-scoped local stores) so mutes
 * from different communities/relays don't bleed across each other — in
 * particular so a non-empty store from relay A can never seed-publish onto a
 * first-visited relay B (Carl P1). When omitted the legacy pubkey-only key is
 * returned (used only during one-time migration in `readChannelMutesStore`).
 */
export function storageKey(pubkey: string, relayUrl?: string): string {
  return makeStorageKey(STORAGE_KEY_PREFIX, pubkey, relayUrl);
}

export function parseMutePayload(json: unknown): ChannelMuteStore | null {
  if (typeof json !== "object" || json === null) return null;
  const obj = json as Record<string, unknown>;
  if (obj.version !== 1) return null;
  const channels: Record<string, ChannelMuteEntry> =
    typeof obj.channels === "object" &&
    obj.channels !== null &&
    !Array.isArray(obj.channels)
      ? Object.fromEntries(
          Object.entries(obj.channels as Record<string, unknown>)
            .filter((entry): entry is [string, Record<string, unknown>] => {
              const v = entry[1];
              return (
                typeof v === "object" &&
                v !== null &&
                typeof (v as Record<string, unknown>).muted === "boolean" &&
                typeof (v as Record<string, unknown>).updatedAt === "number" &&
                Number.isSafeInteger(
                  (v as Record<string, unknown>).updatedAt as number,
                ) &&
                ((v as Record<string, unknown>).updatedAt as number) >= 0
              );
            })
            // Normalize `rev`: accept a non-negative integer with headroom for
            // one mint, otherwise 0. An entry is never dropped solely because
            // `rev` is absent (older build) or malformed — absence is a valid
            // mergeable value. `rev >= Number.MAX_SAFE_INTEGER` is rejected:
            // the click path mints `maxRev + 1`, so accepting the boundary
            // itself would overflow to the same unsafe value and stop
            // advancing, wedging every later toggle. Requiring strictly-below
            // keeps `maxRev + 1` safe. Treating an out-of-range value as rev 0
            // lets a real later click win the same-second tie instead (Carl P2).
            .map(([id, v]) => {
              const rawRev = v.rev;
              const rev =
                typeof rawRev === "number" &&
                Number.isSafeInteger(rawRev) &&
                rawRev >= 0 &&
                rawRev < Number.MAX_SAFE_INTEGER
                  ? rawRev
                  : 0;
              return [
                id,
                {
                  muted: v.muted as boolean,
                  updatedAt: v.updatedAt as number,
                  rev,
                },
              ];
            }),
        )
      : {};
  return boundMuteStore({ version: 1, channels });
}

function parseRaw(raw: string | null): ChannelMuteStore | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null || parsed.version !== 1) {
      return null;
    }
    return parseMutePayload(parsed);
  } catch {
    return null;
  }
}

/**
 * Read the mute store for `pubkey` scoped to `relayUrl`.
 *
 * Enforces one invariant on every read: a scoped copy is exposed only while no
 * importable legacy (pubkey-only) key remains. If a non-empty legacy key still
 * exists — whether because this is the first scoped read or because a prior
 * migration wrote the scoped copy but could not delete the legacy key — we
 * finish the claim inline: write the scoped copy if absent, delete the legacy
 * key, and expose the value only once that delete provably took. On any storage
 * failure we return `DEFAULT_STORE`, which cannot seed-publish (bootstrap gates
 * on a non-empty local store), leaving the claim to complete on a later read
 * once storage recovers. This makes "scoped data is owned only when the legacy
 * key is gone" a property the reader proves every read, not a promise a
 * one-shot migration hopes to keep under failing storage — so relay A's mutes
 * can never seed-publish onto a first-visited relay B (Carl P1).
 *
 * Concurrent windows are safe without an atomic claim. Each window (main and
 * huddle) takes its scope from the app-wide active community at mount, so the
 * first scoped read for a live legacy key runs before any second relay scope
 * can exist; that read either claims the legacy key or hits the failure path
 * above. A race migrates the identical legacy value to the identical scoped key
 * (idempotent), and the loser's post-delete confirmation fails and exposes
 * `DEFAULT_STORE`.
 */
export function readChannelMutesStore(
  pubkey: string,
  relayUrl?: string,
): ChannelMuteStore {
  return readStore(
    STORAGE_KEY_PREFIX,
    pubkey,
    relayUrl,
    parseRaw,
    DEFAULT_STORE,
  );
}

export function boundMuteStore(
  store: ChannelMuteStore,
  preservedKey?: string,
): ChannelMuteStore {
  const preservedEntry =
    preservedKey === undefined ? undefined : store.channels[preservedKey];
  const entries = Object.entries(store.channels).filter(
    ([channelId]) => channelId !== preservedKey,
  );
  if (entries.length + (preservedEntry ? 1 : 0) <= MAX_CHANNEL_MUTE_ENTRIES)
    return store;
  entries.sort(([leftId, left], [rightId, right]) => {
    if (left.updatedAt !== right.updatedAt)
      return left.updatedAt - right.updatedAt;
    return leftId < rightId ? -1 : leftId > rightId ? 1 : 0;
  });
  const retainedEntries = entries.slice(
    -(MAX_CHANNEL_MUTE_ENTRIES - (preservedEntry ? 1 : 0)),
  );
  if (preservedEntry && preservedKey !== undefined) {
    retainedEntries.push([preservedKey, preservedEntry]);
  }
  return {
    ...store,
    channels: Object.fromEntries(retainedEntries),
  };
}

/**
 * Persist the main store. Writes the passed store as-is (bounded) — no read of
 * the shared key, so it is never a shared-key read-modify-write. Callers merge
 * peer state into the window's OWN React state (via the storage-event handler
 * and applyRemote) before calling here, so the write carries an owned, merged
 * value. Returns the bounded store, or `null` on write failure.
 *
 * Cross-window convergence of the on-disk cache is eventual: a peer's storage
 * event folds into this window's state, and the relay reconcile writes the
 * merged head back. Durable no-loss of an unpublished click is held by the
 * per-window outbox, not this cache.
 */
export function writeChannelMutesStore(
  pubkey: string,
  store: ChannelMuteStore,
  relayUrl?: string,
  preservedKey?: string,
): ChannelMuteStore | null {
  try {
    const bounded = boundMuteStore(store, preservedKey);
    window.localStorage.setItem(
      storageKey(pubkey, relayUrl),
      JSON.stringify(bounded),
    );
    return bounded;
  } catch {
    return null;
  }
}

/**
 * Merge two mute stores by a per-channel total order:
 * `updatedAt` DESC → `rev` DESC → `muted === true` wins. This order is
 * commutative, associative, and idempotent (before bounding), so every
 * observation path (bootstrap, live, reconnect, reconcile, pre-publish,
 * cross-window storage) applies it with no ordering or ownership overlay and
 * all replicas converge.
 *
 * `updatedAt` is primary so a strictly-later edit — from any build, whether it
 * carries `rev` or (older build) reads `rev: 0` — wins outright. `rev` breaks
 * only a same-second `updatedAt` tie: the ambiguous integer-second window the
 * clock cannot resolve, where a NEW-build click that minted `rev = maxSeen + 1`
 * dominates same-second state it observed. On a full tie (equal `updatedAt` AND
 * equal `rev`) `true` wins as the deterministic leaf.
 *
 * ACCEPTED MIXED-FLEET RESIDUAL (Carl P1 #5, no protocol change): an OLD build
 * mints no `rev`, so its click reads `rev: 0`. A later old-build click that
 * lands in the SAME integer second as an earlier new-build write carrying
 * `rev >= 1` therefore loses that same-second tie — the old click's intent is
 * suppressed until the user clicks again in a strictly-later second, when the
 * primary `updatedAt` key carries it. This is bounded (one integer second,
 * old-build writer only, self-heals on the next later-second click) and is no
 * worse than the shipped LWW behavior; closing it would need a version
 * migration we deliberately do not build. `mergeStores: same-second old-build
 * click (rev 0) loses to an earlier new-build rev, heals next second` pins the
 * exact current outcome.
 */
export function mergeStores(
  a: ChannelMuteStore,
  b: ChannelMuteStore,
  preservedKey?: string,
): ChannelMuteStore {
  const allIds = new Set([
    ...Object.keys(a.channels),
    ...Object.keys(b.channels),
  ]);
  const merged: Record<string, ChannelMuteEntry> = {};
  for (const id of allIds) {
    const l = a.channels[id];
    const r = b.channels[id];
    merged[id] = l && r ? pickMuteEntry(l, r) : ((l ?? r) as ChannelMuteEntry);
  }
  return boundMuteStore({ version: 1, channels: merged }, preservedKey);
}

/** The winner of two entries under `updatedAt` → `rev` → `muted` order. */
function pickMuteEntry(
  l: ChannelMuteEntry,
  r: ChannelMuteEntry,
): ChannelMuteEntry {
  if (l.updatedAt !== r.updatedAt) return l.updatedAt > r.updatedAt ? l : r;
  if (l.rev !== r.rev) return l.rev > r.rev ? l : r;
  if (l.muted !== r.muted) return l.muted ? l : r;
  return l;
}

export function mutedChannelIdsFromStore(store: ChannelMuteStore): Set<string> {
  return new Set(
    Object.entries(store.channels)
      .filter(([, entry]) => entry.muted)
      .map(([id]) => id),
  );
}

/**
 * Persist this window's unpublished edit under its own outbox key. Written
 * synchronously on every click as a single unconditional `setItem` (no shared-
 * key read-modify-write); resumed by merging every window's record on next
 * mount so a click made <2s before quit/community-switch is never dropped.
 */
export function writeChannelMutesOutbox(
  pubkey: string,
  store: ChannelMuteStore,
  relayUrl: string,
  preservedKey?: string,
): void {
  writeOutbox(
    OUTBOX_KEY_PREFIX,
    pubkey,
    store,
    relayUrl,
    boundMuteStore,
    preservedKey,
  );
}

/**
 * Merge every window's persisted unpublished edit into one store for resume,
 * together with the most-recent explicit `preservedKey` across all records
 * (own and foreign), or null when no records exist.
 *
 * The preserved key is selected deterministically: the record with the highest
 * `queuedAt` that carries an explicit `preservedKey`; ties broken by storage
 * key string (max). This recovers the durable reservation after a quit and
 * restart, when the prior window's record is now foreign (its session nonce is
 * gone) but its `preservedKey` is still in localStorage (Kalvin P3).
 *
 * Both the fold and the capacity bound apply the selected key so the clicked
 * channel is never evicted during the read itself.
 */
export function readChannelMutesOutboxWithMeta(
  pubkey: string,
  relayUrl: string,
): OutboxWithMeta<ChannelMuteStore> | null {
  return readOutboxWithMeta(
    OUTBOX_KEY_PREFIX,
    pubkey,
    relayUrl,
    parseMutePayload,
    mergeStores,
    DEFAULT_STORE,
    boundMuteStore,
  );
}

/** Clear this window's own outbox key (its edit published or is a no-op). */
export function clearChannelMutesOutbox(
  pubkey: string,
  relayUrl: string,
): void {
  clearOutbox(OUTBOX_KEY_PREFIX, pubkey, relayUrl);
}

/**
 * True when the fetched relay `head` already reflects every entry in
 * `candidate` — merging the candidate into the head leaves it unchanged. Used
 * both to reclaim a subsumed foreign key and to skip a redundant boot-time
 * replay publish of a fold the head already carries (e.g. only the
 * never-deleted legacy key lingers).
 *
 * When `preservedKey` is provided the check is precise: `head` must contain an
 * entry for `preservedKey` with `rev >= candidate.channels[preservedKey].rev`.
 * This avoids using capacity-bounded `mergeStores` as the subsumption proof:
 * without a preserved key, mergeStores can evict the clicked entry at the
 * 500-cap boundary and return the truncated head unchanged, incorrectly
 * certifying retention of a click the relay never kept (Carl P3).
 *
 * When `preservedKey` is not provided (no click reservation active) the
 * bounded merge proof is still semantically safe — no specific entry needs to
 * survive eviction — so we fall back to the merge-equality check.
 */
export function isMutesStoreSubsumedBy(
  candidate: ChannelMuteStore,
  head: ChannelMuteStore,
  preservedKey?: string,
): boolean {
  if (preservedKey !== undefined) {
    const candidateEntry = candidate.channels[preservedKey];
    if (candidateEntry === undefined) {
      return muteStoresEqual(mergeStores(head, candidate), head);
    }
    const headEntry = head.channels[preservedKey];
    if (!headEntry) return false;
    if (headEntry.updatedAt < candidateEntry.updatedAt) return false;
    if (
      headEntry.updatedAt === candidateEntry.updatedAt &&
      headEntry.rev < candidateEntry.rev
    )
      return false;
    return muteStoresEqual(mergeStores(head, candidate, preservedKey), head);
  }
  return muteStoresEqual(mergeStores(head, candidate), head);
}

/**
 * Reclaim foreign outbox keys the fetched relay head already subsumes: a record
 * is redundant when merging it into `head` yields `head` unchanged (the head
 * carries an entry at least as new for every channel). Never touches this
 * window's own key; a still-unpublished peer edit the head does not yet reflect
 * is kept. Call only after a successful head fetch.
 */
export function reclaimSubsumedMutesOutbox(
  pubkey: string,
  relayUrl: string,
  head: ChannelMuteStore,
): void {
  reclaimSubsumedOutbox(
    OUTBOX_KEY_PREFIX,
    pubkey,
    relayUrl,
    parseMutePayload,
    isMutesStoreSubsumedBy,
    head,
  );
}

/** Deep per-channel equality of two mute stores (order-independent). */
function muteStoresEqual(a: ChannelMuteStore, b: ChannelMuteStore): boolean {
  const aKeys = Object.keys(a.channels);
  const bKeys = Object.keys(b.channels);
  if (aKeys.length !== bKeys.length) return false;
  for (const id of aKeys) {
    const l = a.channels[id];
    const r = b.channels[id];
    if (
      !r ||
      l.muted !== r.muted ||
      l.updatedAt !== r.updatedAt ||
      l.rev !== r.rev
    )
      return false;
  }
  return true;
}
