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
   * Lifecycle status of this draft.
   * - "active": draft is in progress (not yet sent).
   * - "sent": draft was sent; kept for the Drafts inbox "Sent" subsection.
   * Entries persisted before this field was added have no status field —
   * the read path treats absent status as "active" (see `isValidDraftState`).
   */
  status: "active" | "sent";
};

/** Serialised shape stored in localStorage (same as DraftState for round-trips). */
type StoredDrafts = Record<string, DraftState>;

const DRAFT_STORE_KEY_PREFIX = "buzz-drafts.v1";
const MAX_DRAFTS = 100;

/** Module-level pubkey set by `initDraftStore`. Empty string = no identity. */
let currentPubkey = "";

/** Monotonically-incrementing counter used to guarantee unique sent-record keys
 *  even when two sends happen within the same millisecond (e.g. in tests). */
let _sentSeq = 0;

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
  // status. Treat absent/invalid status as "active" rather than rejecting the
  // entry — this avoids data loss on first run after the upgrade.
  if (d.status === undefined || d.status === null) {
    (d as DraftState).status = "active";
  } else if (d.status !== "active" && d.status !== "sent") {
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
 * Used by the "Sent" subsection of the Drafts inbox panel.
 */
export function getSentDraftEntries(): Array<{
  key: string;
  draft: DraftState;
}> {
  return getAllDraftEntries().filter((e) => e.draft.status === "sent");
}

/**
 * Mark a draft as sent by writing its content to a durable sent-record key.
 *
 * The active draft key is simultaneously cleared so the composer can create
 * a fresh draft in the same channel without inheriting the sent status, and so
 * the composer's empty-content cleanup can never delete the sent record.
 *
 * The sent record is stored under `sent:<draftKey>:<timestamp>` — a key the
 * composer never writes to — so active and sent records for the same channel
 * can coexist in the store independently.
 *
 * The "never-persisted draft writes no sent record" boundary is enforced at
 * the call site: callers only invoke this function when `sentDraftKey` is
 * non-null, which only holds for drafts that were persisted before submit.
 * This function writes unconditionally so the sent record is created even
 * when the active key was already cleared by a composer cleanup that raced
 * the async send (e.g. the user switched channels while send was in flight).
 */
export function markDraftSentEntry(
  draftKey: string,
  content: string,
  channelId: string,
  pendingImeta: ImetaMedia[],
  spoileredAttachmentUrls: string[],
): void {
  const map = readStore();
  const existing = map.get(draftKey);
  const now = new Date().toISOString();
  // Use the live entry's createdAt when available; fall back to now when the
  // active key was already cleared by a navigation-during-send race. Either
  // way the sent record is written — the race cannot cause data loss.
  const createdAt = existing?.createdAt ?? now;
  // Write the sent record under a stable, distinct key so it can never be
  // overwritten by the composer's active-draft persist path.
  // The `Date.now()-seq` suffix guarantees uniqueness even if two sends in the
  // same channel happen within the same millisecond.
  const sentKey = `sent:${draftKey}:${Date.now()}-${++_sentSeq}`;
  map.set(sentKey, {
    content,
    selectionStart: content.length,
    selectionEnd: content.length,
    channelId,
    createdAt,
    updatedAt: now,
    pendingImeta,
    spoileredAttachmentUrls,
    status: "sent",
  });

  // Clear the active draft key (if still present) so the composer starts fresh
  // and any subsequent empty-content persist doesn't encounter the sent record.
  map.delete(draftKey);
  evictOldest(map);
  flushStore(map);
  notifySubscribers();
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
