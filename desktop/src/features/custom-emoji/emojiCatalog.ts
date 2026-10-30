import {
  EMOJI_AUTOCOMPLETE_DEBOUNCE_MS,
  EMOJI_AUTOCOMPLETE_RESULT_CAP,
  EMOJI_PICKER_RESULT_CAP,
  createEmojiResolver,
  createPrefixSearcher,
} from "@/features/custom-emoji/emojiCatalogSource";
import { resetEmojiUsage } from "@/features/custom-emoji/emojiUsageStore";
import {
  EMOJI_ENTRY_RESOLVE_MAX_NAMES,
  fetchEmojiEntriesByName,
  searchEmojiEntries,
} from "@/shared/api/emojiEntries";

/** Name lookups for the send path, paste, and node conversion. */
export const emojiResolver = createEmojiResolver({
  fetch: async (names) => (await fetchEmojiEntriesByName(names)).emoji,
  maxNames: EMOJI_ENTRY_RESOLVE_MAX_NAMES,
});

/** Debounced `:par` autocomplete search. */
export const emojiAutocompleteSearcher = createPrefixSearcher({
  fetch: searchEmojiEntries,
  debounceMs: EMOJI_AUTOCOMPLETE_DEBOUNCE_MS,
  resultCap: EMOJI_AUTOCOMPLETE_RESULT_CAP,
  onFresh: (results) => emojiResolver.remember(results),
});

/** Picker search panel. Separate fence so typing in one surface never cancels the other. */
export const emojiPickerSearcher = createPrefixSearcher({
  fetch: searchEmojiEntries,
  debounceMs: EMOJI_AUTOCOMPLETE_DEBOUNCE_MS,
  resultCap: EMOJI_PICKER_RESULT_CAP,
  onFresh: (results) => emojiResolver.remember(results),
});

/** Drop every community-scoped catalog cache. Wired into `resetCommunityState()`. */
export function resetEmojiCatalog(): void {
  emojiResolver.reset();
  emojiAutocompleteSearcher.reset();
  emojiPickerSearcher.reset();
  resetEmojiUsage();
}
