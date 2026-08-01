/**
 * Shared key-dispatch shape for the composer's three autocompletes (emoji,
 * channel link, mention).
 *
 * Split out of `MessageComposer.tsx` to keep that module under the desktop
 * file-size ceiling (see `desktop/scripts/check-file-sizes.mjs`).
 *
 * Each autocomplete's `handle*KeyDown` returns the same contract: whether it
 * consumed the key, and optionally the suggestion the key committed. A
 * consumed key must stop dispatch even when no suggestion was committed —
 * arrow keys move the highlight without selecting anything.
 */
export type AutocompleteKeyResult<T> = {
  handled: boolean;
  suggestion?: T | null;
};

/** Returns true when the autocomplete consumed the key and dispatch should stop. */
export function applyAutocompleteKeyResult<T>(
  result: AutocompleteKeyResult<T>,
  apply: (suggestion: T) => void,
): boolean {
  if (!result.handled) return false;
  if (result.suggestion) apply(result.suggestion);
  return true;
}
