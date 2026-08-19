/**
 * Resolves the sentDraftKey to pass to sendMessageWithMentionFlow at submit
 * time.
 *
 * Returns the key when a draft was actually persisted in the store before the
 * send fired (so the active draft should be cleared on send), or null when no
 * entry exists (fast / never-persisted send — no active draft to clear).
 *
 * This is a pure function with no dependencies so it can be imported and
 * exercised directly in Node .mjs tests without a React renderer.
 *
 * @param effectiveDraftKey - the draft key captured synchronously at submit time
 * @param loadDraft - synchronous O(1) store read; returns undefined when absent
 */
export function resolveSentDraftKey(
  effectiveDraftKey: string | null | undefined,
  loadDraft: (key: string) => unknown,
): string | null {
  return effectiveDraftKey && loadDraft(effectiveDraftKey)
    ? effectiveDraftKey
    : null;
}
