// Per-scope composer drafts: text typed into one channel's composer must not
// follow the user to another channel. Keyed by channel id, or FRESH_DRAFT_KEY
// for the fresh/navigator new-session composer. Module-level so drafts
// survive display-style toggles (which unmount the dev shell); intentionally
// in-memory only — drafts do not persist across app restarts.
const drafts = new Map<string, string>();

export const FRESH_DRAFT_KEY = "fresh";

export function loadComposerDraft(key: string): string {
  return drafts.get(key) ?? "";
}

/** Whitespace-only text is not a draft — it clears the slot. */
export function saveComposerDraft(key: string, text: string): void {
  if (text.trim().length === 0) {
    drafts.delete(key);
  } else {
    drafts.set(key, text);
  }
}

/** Enter consumed the text — the scope no longer has a draft. */
export function consumeComposerDraft(key: string): void {
  drafts.delete(key);
}

/**
 * A failed send restores its prompt to the scope it was sent from — but
 * never over text the user has since drafted there.
 */
export function saveComposerDraftIfEmpty(key: string, text: string): void {
  if (loadComposerDraft(key) === "") {
    saveComposerDraft(key, text);
  }
}

/** Tests only — the module-level map would otherwise leak across cases. */
export function clearComposerDrafts(): void {
  drafts.clear();
}
