import * as React from "react";

import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";

// ── Store reactivity ─────────────────────────────────────────────────────────
// `useSyncExternalStore` requires a stable subscribe/getSnapshot pair.
// localStorage is not reactive, so we maintain a module-level subscriber set
// and version counter. Every write bumps the counter and notifies subscribers
// so any component consuming `useDraftsSnapshot()` re-renders immediately.

type Subscriber = () => void;
const _subscribers = new Set<Subscriber>();
let _version = 0;

/** Notify all active subscribers. Called by every write path. */
function notifySubscribers(): void {
  _version += 1;
  for (const sub of _subscribers) {
    sub();
  }
}

function subscribeToStore(callback: Subscriber): () => void {
  _subscribers.add(callback);
  return () => {
    _subscribers.delete(callback);
  };
}

function getStoreSnapshot(): number {
  return _version;
}

/**
 * Subscribe to draft store changes. Returns an unsubscribe function.
 * Exported for unit-testing the subscriber notification contract.
 * Use `useDraftsSnapshot()` in React components.
 */
export { subscribeToStore, getStoreSnapshot };

export type DraftState = {
  content: string;
  selectionStart: number;
  selectionEnd: number;
  /**
   * The channel (or thread-scoped) ID this draft belongs to.
   * Stored explicitly — do NOT parse the draft key to recover it.
   * Thread draft keys use the form `thread:${threadHead.id}`; the
   * channelId is the containing channel.
   */
  channelId: string;
  /** ISO-8601 timestamp when this draft was first created. */
  createdAt: string;
  /** ISO-8601 timestamp when this draft was last updated. */
  updatedAt: string;
  /** Pasted/uploaded image attachments, preserved across channel-switch. */
  pendingImeta: ImetaMedia[];
  /** URLs of imeta attachments marked as spoilered. */
  spoileredAttachmentUrls: string[];
  /**
   * Lifecycle status of this draft. Always `"active"` at runtime.
   * The `"sent"` value is not written by any production path; legacy `sent:`
   * keyed records from older builds are dropped on read by `readStore`.
   * Entries persisted before this field was added have no status field —
   * the read path treats absent status as `"active"` (see `isValidDraftState`).
   */
  status: "active" | "sent";
};

/** Serialised shape stored in localStorage (same as DraftState for round-trips). */
type StoredDrafts = Record<string, DraftState>;

const DRAFT_STORE_KEY_PREFIX = "buzz-drafts.v1";
const MAX_DRAFTS = 100;

/** Module-level pubkey set by `initDraftStore`. Empty string = no identity. */
let currentPubkey = "";

function storageKey(): string {
  return `${DRAFT_STORE_KEY_PREFIX}:${currentPubkey}`;
}

/**
 * Initialise (or re-initialise) the draft store for a given identity.
 * Called from `useWorkspaceInit` alongside the other singleton resets.
 * Resets the in-memory cache whenever the pubkey changes so a direct
 * identity switch (without a prior `clearAllDrafts`) never serves the
 * wrong identity's drafts.
 */
export function initDraftStore(pubkey: string): void {
  if (currentPubkey !== pubkey) {
    _memCache = null;
  }
  currentPubkey = pubkey;
  // Eagerly load to surface corruption errors in console at startup rather
  // than on first draft interaction.
  readStore();
}

/**
 * Reset the in-memory draft store on workspace switch.
 * Replaces the old `clearAllDrafts()`.
 */
export function clearAllDrafts(): void {
  currentPubkey = "";
  _memCache = null;
}

// ── In-memory write-back cache ────────────────────────────────────────────────
// We keep a parsed copy so reads are synchronous O(1) object lookups,
// and only flush to localStorage on writes.

let _memCache: Map<string, DraftState> | null = null;

function readStore(): Map<string, DraftState> {
  if (_memCache !== null) return _memCache;

  const map = new Map<string, DraftState>();
  if (!currentPubkey) {
    _memCache = map;
    return map;
  }

  const raw = localStorage.getItem(storageKey());
  if (!raw) {
    _memCache = map;
    return map;
  }

  try {
    const parsed: unknown = JSON.parse(raw);
    if (
      parsed !== null &&
      typeof parsed === "object" &&
      !Array.isArray(parsed)
    ) {
      for (const [key, value] of Object.entries(parsed as StoredDrafts)) {
        // Drop legacy sent: records — they were written by the old
        // markDraftSentEntry and have no role now that the Sent section is
        // removed. Skipping here compacts them out on the next flush.
        if (key.startsWith("sent:")) {
          continue;
        }
        if (isValidDraftState(value)) {
          map.set(key, value);
        }
      }
    }
  } catch (err) {
    console.debug("[useDrafts] localStorage corrupt, starting fresh:", err);
  }

  _memCache = map;
  return map;
}

function isValidDraftState(v: unknown): v is DraftState {
  if (typeof v !== "object" || v === null) return false;
  const d = v as Partial<DraftState>;
  if (
    typeof d.content !== "string" ||
    typeof d.selectionStart !== "number" ||
    typeof d.selectionEnd !== "number" ||
    typeof d.channelId !== "string" ||
    typeof d.createdAt !== "string" ||
    typeof d.updatedAt !== "string" ||
    !Array.isArray(d.pendingImeta) ||
    !Array.isArray(d.spoileredAttachmentUrls)
  ) {
    return false;
  }
  // Migration: entries written before the status field was introduced have no
  // status field. Treat absent status as "active" to avoid data loss on the
  // first run after the upgrade.
  // Legacy sent: keys are skipped by readStore before reaching this function;
  // reject any remaining entry whose status is not "active".
  if (d.status === undefined || d.status === null) {
    (d as DraftState).status = "active";
  } else if (d.status !== "active") {
    return false;
  }
  return true;
}

function flushStore(map: Map<string, DraftState>): void {
  if (!currentPubkey) return;
  const obj: StoredDrafts = {};
  for (const [k, v] of map) {
    obj[k] = v;
  }
  setLocalStorageItemWithRecovery(storageKey(), JSON.stringify(obj));
}

/**
 * Evict the least-recently-updated entry until the map is within `MAX_DRAFTS`.
 */
function evictOldest(map: Map<string, DraftState>): void {
  if (map.size <= MAX_DRAFTS) return;
  // Sort ascending by updatedAt; evict oldest until within cap.
  const sorted = [...map.entries()].sort((a, b) =>
    a[1].updatedAt.localeCompare(b[1].updatedAt),
  );
  const excess = map.size - MAX_DRAFTS;
  for (let i = 0; i < excess; i++) {
    map.delete(sorted[i][0]);
  }
}

// ── Public API ────────────────────────────────────────────────────────────────
// The standalone functions below are the primary storage layer. `useDrafts()`
// wraps them in `React.useCallback` for component use; the functions are also
// exported directly so non-React callers (tests, future inbox features) can
// use them without a React context.

export function saveDraftEntry(draftKey: string, draft: DraftState): void {
  if (draft.content.trim().length === 0 && draft.pendingImeta.length === 0) {
    return;
  }
  const map = readStore();
  map.set(draftKey, draft);
  evictOldest(map);
  flushStore(map);
  notifySubscribers();
}

export function loadDraftEntry(draftKey: string): DraftState | undefined {
  return readStore().get(draftKey);
}

export function clearDraftEntry(draftKey: string): void {
  const map = readStore();
  if (map.has(draftKey)) {
    map.delete(draftKey);
    flushStore(map);
    notifySubscribers();
  }
}

/**
 * Return true only when every field of two DraftState values is identical,
 * including all BlobDescriptor optional fields (dim, blurhash, thumb,
 * duration, image, filename, uploaded). Any divergence — including selection
 * offsets, timestamps, attachment metadata, spoiler state, and status — is
 * treated as a distinct record that must not be discarded.
 */
function draftStatesEqual(a: DraftState, b: DraftState): boolean {
  if (
    a.content !== b.content ||
    a.selectionStart !== b.selectionStart ||
    a.selectionEnd !== b.selectionEnd ||
    a.channelId !== b.channelId ||
    a.createdAt !== b.createdAt ||
    a.updatedAt !== b.updatedAt ||
    a.status !== b.status ||
    a.pendingImeta.length !== b.pendingImeta.length ||
    a.spoileredAttachmentUrls.length !== b.spoileredAttachmentUrls.length
  ) {
    return false;
  }
  for (let i = 0; i < a.pendingImeta.length; i++) {
    const am = a.pendingImeta[i];
    const bm = b.pendingImeta[i];
    if (
      am.url !== bm.url ||
      am.sha256 !== bm.sha256 ||
      am.size !== bm.size ||
      am.type !== bm.type ||
      am.uploaded !== bm.uploaded ||
      am.dim !== bm.dim ||
      am.blurhash !== bm.blurhash ||
      am.thumb !== bm.thumb ||
      am.duration !== bm.duration ||
      am.image !== bm.image ||
      am.filename !== bm.filename
    ) {
      return false;
    }
  }
  for (let i = 0; i < a.spoileredAttachmentUrls.length; i++) {
    if (a.spoileredAttachmentUrls[i] !== b.spoileredAttachmentUrls[i]) {
      return false;
    }
  }
  return true;
}

/**
 * Atomically rename a draft key: move the DraftState from `oldKey` to
 * `newKey`, flush once, and notify once. Returns the outcome:
 *
 * - `"migrated"` — the rename succeeded; `oldKey` no longer exists.
 * - `"collision"` — `newKey` already held a record whose full persisted
 *   DraftState differed from `oldKey`'s record across any field (content,
 *   selection, timestamps, attachments, spoiler state, or status). Both
 *   records are preserved unchanged; no I/O is performed.
 * - `"noop"` — `oldKey` did not exist in the store; nothing changed.
 *
 * If `oldKey === newKey` no I/O occurs and `"noop"` is returned.
 * Map cardinality is unchanged by a rename (one key removed, one added or
 * identical collapse), so `evictOldest` is never called here.
 * Callers must not compose this from public save+clear calls (that would
 * issue two flushes and could overwrite a concurrent write).
 */
export function renameDraftEntry(
  oldKey: string,
  newKey: string,
): "migrated" | "collision" | "noop" {
  if (oldKey === newKey) return "noop";
  const map = readStore();
  const existing = map.get(oldKey);
  if (existing === undefined) return "noop";

  const destination = map.get(newKey);
  if (destination !== undefined) {
    // Only collapse when every persisted field is identical; any divergence
    // is treated as a collision to prevent silent data loss.
    if (!draftStatesEqual(existing, destination)) {
      return "collision";
    }
    // Identical records: remove the legacy key, keep the destination entry.
    map.delete(oldKey);
    flushStore(map);
    notifySubscribers();
    return "migrated";
  }

  // No destination conflict: move the record. Cardinality is unchanged
  // (one delete + one set), so evictOldest is not called.
  map.set(newKey, existing);
  map.delete(oldKey);
  flushStore(map);
  notifySubscribers();
  return "migrated";
}

/**
 * Convenience: save if content or attachments are non-empty, otherwise clear.
 * Preserves existing createdAt on updates; sets it on first save.
 */
export function persistDraftEntry(
  draftKey: string,
  content: string,
  channelId: string,
  pendingImeta: ImetaMedia[],
  spoileredAttachmentUrls: string[],
): void {
  const hasContent = content.trim().length > 0 || pendingImeta.length > 0;
  if (hasContent) {
    const map = readStore();
    const existing = map.get(draftKey);
    const now = new Date().toISOString();
    saveDraftEntry(draftKey, {
      content,
      selectionEnd: content.length,
      selectionStart: content.length,
      channelId,
      createdAt: existing?.createdAt ?? now,
      updatedAt: now,
      pendingImeta,
      spoileredAttachmentUrls,
      status: "active",
    });
  } else {
    clearDraftEntry(draftKey);
  }
}

/**
 * Returns all drafts sorted most-recently-updated first.
 * Used by the Drafts inbox panel (Phase 2).
 */
export function getAllDraftEntries(): Array<{
  key: string;
  draft: DraftState;
}> {
  return [...readStore().entries()]
    .sort((a, b) => b[1].updatedAt.localeCompare(a[1].updatedAt))
    .map(([key, draft]) => ({ key, draft }));
}

/**
 * Returns only active (unsent) drafts, sorted most-recently-updated first.
 * Used by the "Drafts" subsection of the Drafts inbox panel.
 */
export function getActiveDraftEntries(): Array<{
  key: string;
  draft: DraftState;
}> {
  return getAllDraftEntries().filter((e) => e.draft.status === "active");
}

/**
 * Returns only sent drafts, sorted most-recently-updated first.
 * Returns empty — sent records are dropped on read. Kept for test assertions.
 */
export function getSentDraftEntries(): Array<{
  key: string;
  draft: DraftState;
}> {
  return getAllDraftEntries().filter((e) => e.draft.status === "sent");
}

/**
 * Clear the active draft entry for a sent draft.
 *
 * Kept as a named export so callers (`useMentionSendFlow`) don't need
 * updating. Previously wrote a visible sent-record to the store; the
 * sent section has been removed, so we now just clear the active draft.
 */
export function markDraftSentEntry(
  draftKey: string,
  _content: string,
  _channelId: string,
  _pendingImeta: ImetaMedia[],
  _spoileredAttachmentUrls: string[],
): void {
  clearDraftEntry(draftKey);
}

// ── Reactive hooks ────────────────────────────────────────────────────────────

/**
 * Returns the current store version, re-rendering the component on every
 * draft write (save / clear / persist / markSent). The version number itself
 * is not meaningful — callers derive their actual data from the snapshot.
 *
 * Use this anywhere that needs to react to draft changes without polling:
 * - `DraftsPanel` (replaces manual `refreshDrafts` + `useEffect`)
 * - `useActiveDraftCount` (badge count)
 */
export function useDraftsSnapshot(): number {
  return React.useSyncExternalStore(subscribeToStore, getStoreSnapshot);
}

export function useDrafts() {
  const saveDraft = React.useCallback(
    (draftKey: string, draft: DraftState) => saveDraftEntry(draftKey, draft),
    [],
  );

  const loadDraft = React.useCallback(
    (draftKey: string): DraftState | undefined => loadDraftEntry(draftKey),
    [],
  );

  const clearDraft = React.useCallback(
    (draftKey: string) => clearDraftEntry(draftKey),
    [],
  );

  const persistDraft = React.useCallback(
    (
      draftKey: string,
      content: string,
      channelId: string,
      pendingImeta: ImetaMedia[],
      spoileredAttachmentUrls: string[],
    ) =>
      persistDraftEntry(
        draftKey,
        content,
        channelId,
        pendingImeta,
        spoileredAttachmentUrls,
      ),
    [],
  );

  const getAllDrafts = React.useCallback(() => getAllDraftEntries(), []);

  const getActiveDrafts = React.useCallback(() => getActiveDraftEntries(), []);

  const getSentDrafts = React.useCallback(() => getSentDraftEntries(), []);

  const markDraftSent = React.useCallback(
    (
      draftKey: string,
      content: string,
      channelId: string,
      pendingImeta: ImetaMedia[],
      spoileredAttachmentUrls: string[],
    ) =>
      markDraftSentEntry(
        draftKey,
        content,
        channelId,
        pendingImeta,
        spoileredAttachmentUrls,
      ),
    [],
  );

  return {
    saveDraft,
    loadDraft,
    clearDraft,
    persistDraft,
    getAllDrafts,
    getActiveDrafts,
    getSentDrafts,
    markDraftSent,
  };
}

export type UseDraftsResult = ReturnType<typeof useDrafts>;
