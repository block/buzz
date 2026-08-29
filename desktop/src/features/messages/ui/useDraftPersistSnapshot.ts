import * as React from "react";

import { stripImplicitAgentMentionPrefix } from "@/features/messages/lib/stripImplicitAgentMentions";

import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import type { QueuedMediaAttachment } from "@/features/messages/lib/backgroundMediaUploadStore";
import {
  getDraftStoreScope,
  type DraftMentionRef,
  type DraftState,
} from "@/features/messages/lib/useDrafts";

type UseDraftPersistLifecycleParams = {
  effectiveDraftKey: string | null | undefined;
  channelId: string | null | undefined;
  /** Load a saved draft from the store. */
  loadDraft: (draftKey: string) => DraftState | undefined;
  /** Persist the current draft to the store (called in effect cleanup). */
  persistDraft: (
    draftKey: string,
    content: string,
    channelId: string,
    pendingImeta: ImetaMedia[],
    spoileredAttachmentUrls: string[],
    mentionRefs: DraftMentionRef[],
  ) => void;
  /** Snapshot selected mention identities still present in current content. */
  getMentionRefs: (content: string) => DraftMentionRef[];
  /** Replace mention routing/highlight state when a draft is restored or cleared. */
  restoreMentionRefs: (refs: readonly DraftMentionRef[]) => void;
  /** Live `pendingImeta` from React state — used for render-time ref sync. */
  livePendingImeta: ImetaMedia[];
  /** Async setter for pendingImeta — called after the synchronous snapshot. */
  setPendingImeta: (imeta: ImetaMedia[]) => void;
  /** Snapshot the local files owned by the outgoing draft key. */
  getQueuedAttachments?: () => QueuedMediaAttachment[];
  /** Retain local files in memory under their draft key. */
  saveQueuedAttachmentsForDraft?: (
    draftKey: string,
    attachments: QueuedMediaAttachment[],
  ) => void;
  /** Local files cannot be persisted, so clear them at a draft-key boundary. */
  clearQueuedAttachments?: () => void;
  /** Restore local files retained while a deferred upload was off-channel. */
  restoreQueuedAttachments?: (attachments: QueuedMediaAttachment[]) => void;
  /** Read and remove local files retained for a recovered draft. */
  takeQueuedAttachmentsForDraft?: (draftKey: string) => QueuedMediaAttachment[];
  /** Set the rich-text editor content from a draft string. */
  setContent: (content: string) => void;
  /** Clear the rich-text editor content (no-draft path). */
  clearContent: () => void;
  /** Set the spoilered attachment URLs state. */
  setSpoileredAttachmentUrls: (urls: Set<string>) => void;
  /**
   * Stable ref to the spoilered attachment URLs — read in the cleanup closure
   * so it always captures the latest value at cleanup time.
   */
  spoileredAttachmentUrlsRef: React.MutableRefObject<Set<string>>;
  /**
   * Read the current editor content synchronously — called in the cleanup
   * closure to capture the latest text before the effect fires.
   */
  syncComposerContentFromEditor: () => string;
  /** Exact editor prefix inserted by automatic addressing, including separator. */
  getImplicitAgentMentionPrefix?: () => string;
};

type UseDraftPersistLifecycleResult = {
  /**
   * Record the latest authored editor content. Empty content is persisted
   * immediately and remains authoritative across composer remounts until a
   * later non-empty editor update supersedes it.
   */
  trackAuthoredContent: (content: string) => void;
};

const authoritativelyClearedDraftKeys = new Set<string>();

function scopedDraftKey(draftKey: string): string {
  return `${getDraftStoreScope()}:${draftKey}`;
}

/**
 * Owns the draft-persist lifecycle for `MessageComposer`.
 *
 * This hook:
 * - Holds `pendingImetaForPersistRef` — the ref the cleanup reads when
 *   persisting `pendingImeta` to the draft store.
 * - Updates that ref on every render (render-time passive path) so normal
 *   add/remove-image operations are always captured.
 * - Runs a `useEffect` keyed on `effectiveDraftKey` that restores a saved
 *   draft into the composer (content + imeta + spoilered urls) or clears it,
 *   and whose cleanup persists the outgoing draft before the key changes.
 *
 * **The StrictMode fix lives here.**
 * When the restore effect body calls `setPendingImeta(saved.pendingImeta)`,
 * that state update is async — it won't commit until React re-renders.
 * React StrictMode (dev builds) simulates an unmount immediately after the
 * effect body, before the re-render. Without the synchronous write the
 * cleanup would read `[]` and overwrite the just-restored images.
 *
 * The effect body calls `snapshotPendingImeta` (the synchronous ref write)
 * BEFORE `setPendingImeta`, so the cleanup always sees the correct value.
 *
 * Extracted from `MessageComposer` so the full lifecycle can be exercised
 * directly in a StrictMode test without mounting the full composer.
 */
export function useDraftPersistLifecycle({
  effectiveDraftKey,
  channelId,
  loadDraft,
  persistDraft,
  getMentionRefs,
  restoreMentionRefs,
  livePendingImeta,
  setPendingImeta,
  getQueuedAttachments,
  saveQueuedAttachmentsForDraft,
  clearQueuedAttachments,
  restoreQueuedAttachments,
  takeQueuedAttachmentsForDraft,
  setContent,
  clearContent,
  setSpoileredAttachmentUrls,
  spoileredAttachmentUrlsRef,
  syncComposerContentFromEditor,
  getImplicitAgentMentionPrefix,
}: UseDraftPersistLifecycleParams): UseDraftPersistLifecycleResult {
  const persistedContent = React.useCallback(
    (content: string) =>
      stripImplicitAgentMentionPrefix(
        content,
        getImplicitAgentMentionPrefix?.() ?? "",
      ),
    [getImplicitAgentMentionPrefix],
  );

  const pendingImetaForPersistRef = React.useRef<ImetaMedia[]>([]);
  const emptyContentIsAuthoritativeRef = React.useRef(false);
  const isRestoringContentRef = React.useRef(false);
  const restoredQueuedAttachmentsRef = React.useRef<QueuedMediaAttachment[]>(
    [],
  );
  const restoredQueuedAttachmentsDraftKeyRef = React.useRef<string | null>(
    null,
  );
  // Render-time update: keep the ref in sync with committed state so the
  // cleanup always reads the latest value during normal mounted operation.
  pendingImetaForPersistRef.current = livePendingImeta;

  // biome-ignore lint/correctness/useExhaustiveDependencies: effectiveDraftKey is the sole trigger
  React.useLayoutEffect(() => {
    // The outgoing draft is persisted by the cleanup below, which runs before
    // this body on key changes and has the correct outgoing channelId in its
    // closure. Do NOT re-persist prevKey here: channelId in this render
    // already reflects the incoming channel, which would corrupt the outgoing
    // draft's channelId metadata.

    // Files cannot be serialized into localStorage. Replace the outgoing
    // queue (retained by the cleanup below) with the incoming draft's queue.
    clearQueuedAttachments?.();
    if (effectiveDraftKey !== restoredQueuedAttachmentsDraftKeyRef.current) {
      restoredQueuedAttachmentsDraftKeyRef.current = effectiveDraftKey ?? null;
      restoredQueuedAttachmentsRef.current = effectiveDraftKey
        ? (takeQueuedAttachmentsForDraft?.(effectiveDraftKey) ?? [])
        : [];
    }
    restoreQueuedAttachments?.(restoredQueuedAttachmentsRef.current);
    const authoritativeDraftKey = effectiveDraftKey
      ? scopedDraftKey(effectiveDraftKey)
      : null;
    const wasAuthoritativelyCleared = authoritativeDraftKey
      ? authoritativelyClearedDraftKeys.has(authoritativeDraftKey)
      : false;
    const saved = effectiveDraftKey ? loadDraft(effectiveDraftKey) : undefined;
    emptyContentIsAuthoritativeRef.current = wasAuthoritativelyCleared;
    isRestoringContentRef.current = true;
    if (saved) {
      const restoredContent = wasAuthoritativelyCleared ? "" : saved.content;
      setContent(restoredContent);
      restoreMentionRefs(
        wasAuthoritativelyCleared ? [] : (saved.mentionRefs ?? []),
      );
      // Set the persist-snapshot ref SYNCHRONOUSLY before calling the async
      // state setter, so the cleanup closure (which may fire before the state
      // update commits in React StrictMode's simulate-unmount pass) reads the
      // correct value instead of the stale [].
      pendingImetaForPersistRef.current = saved.pendingImeta;
      setPendingImeta(saved.pendingImeta);
      setSpoileredAttachmentUrls(new Set(saved.spoileredAttachmentUrls));
    } else {
      clearContent();
      restoreMentionRefs([]);
      // Same synchronous snapshot on the empty path.
      pendingImetaForPersistRef.current = [];
      setPendingImeta([]);
      setSpoileredAttachmentUrls(new Set());
    }
    isRestoringContentRef.current = false;

    return () => {
      if (effectiveDraftKey) {
        const queuedAttachments = getQueuedAttachments?.() ?? [];
        if (queuedAttachments.length > 0) {
          saveQueuedAttachmentsForDraft?.(effectiveDraftKey, queuedAttachments);
        }
        const content = emptyContentIsAuthoritativeRef.current
          ? ""
          : persistedContent(syncComposerContentFromEditor());
        persistDraft(
          effectiveDraftKey,
          content,
          channelId ?? effectiveDraftKey,
          [...pendingImetaForPersistRef.current],
          [...spoileredAttachmentUrlsRef.current],
          getMentionRefs(content),
        );
      }
    };
  }, [effectiveDraftKey]);

  const trackAuthoredContent = React.useCallback(
    (content: string) => {
      if (!effectiveDraftKey || isRestoringContentRef.current) return;
      const authoritativeDraftKey = scopedDraftKey(effectiveDraftKey);
      if (content.length > 0) {
        authoritativelyClearedDraftKeys.delete(authoritativeDraftKey);
        emptyContentIsAuthoritativeRef.current = false;
        return;
      }
      authoritativelyClearedDraftKeys.add(authoritativeDraftKey);
      emptyContentIsAuthoritativeRef.current = true;
      persistDraft(
        effectiveDraftKey,
        content,
        channelId ?? effectiveDraftKey,
        [...pendingImetaForPersistRef.current],
        [...spoileredAttachmentUrlsRef.current],
        [],
      );
    },
    [channelId, effectiveDraftKey, persistDraft, spoileredAttachmentUrlsRef],
  );

  return { trackAuthoredContent };
}
