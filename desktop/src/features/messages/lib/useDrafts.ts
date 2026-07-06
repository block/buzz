import * as React from "react";

import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";

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
  return (
    typeof d.content === "string" &&
    typeof d.selectionStart === "number" &&
    typeof d.selectionEnd === "number" &&
    typeof d.channelId === "string" &&
    typeof d.createdAt === "string" &&
    typeof d.updatedAt === "string" &&
    Array.isArray(d.pendingImeta) &&
    Array.isArray(d.spoileredAttachmentUrls)
  );
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
}

export function loadDraftEntry(draftKey: string): DraftState | undefined {
  return readStore().get(draftKey);
}

export function clearDraftEntry(draftKey: string): void {
  const map = readStore();
  if (map.has(draftKey)) {
    map.delete(draftKey);
    flushStore(map);
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

  return { saveDraft, loadDraft, clearDraft, persistDraft, getAllDrafts };
}

export type UseDraftsResult = ReturnType<typeof useDrafts>;
