import * as React from "react";

import { init, SearchIndex } from "emoji-mart";
import data from "@emoji-mart/data";

import { emojiAutocompleteSearcher } from "@/features/custom-emoji/emojiCatalog";
import { EMOJI_AUTOCOMPLETE_RESULT_CAP } from "@/features/custom-emoji/emojiCatalogSource";
import { emojiListKeyAction } from "@/features/custom-emoji/emojiListKeys";
import {
  getEmojiUsageSnapshot,
  recordEmojiUse,
} from "@/features/custom-emoji/emojiUsageStore";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";
import {
  fuzzyStandardEmoji,
  rankByShortcode,
  rankShortcodeMatchesFirst,
} from "@/shared/lib/emojiSearch";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import type { AutocompleteEdit } from "./useRichTextEditor";

export type EmojiSuggestion = {
  id: string;
  name: string;
  native: string;
  /** Set for custom (image) emoji; absent for standard unicode emoji. */
  url?: string;
  /** The unrewritten custom emoji URL, as published. */
  sourceUrl?: string;
};

const EMOJI_DEBOUNCE_MS = 120;
const MIN_QUERY_LENGTH = 2;
const UNLIMITED_RESULTS = Number.POSITIVE_INFINITY;

function toCustomSuggestion(e: CustomEmoji): EmojiSuggestion {
  return {
    id: e.shortcode,
    name: e.shortcode,
    native: "",
    url: rewriteRelayUrl(e.url),
    sourceUrl: e.url,
  };
}

init({ data });

/**
 * Detect an emoji shortcode query at the cursor position.
 * Matches `:query` where `:` is preceded by whitespace or start-of-string,
 * and `query` contains no whitespace or `:`.
 */
function detectEmojiQuery(
  value: string,
  cursorPosition: number,
): { query: string; startIndex: number } | null {
  const beforeCursor = value.slice(0, cursorPosition);
  const match = beforeCursor.match(/(?:^|[\s])(:([^\s:]{2,})?)$/);
  if (!match) return null;

  const full = match[1]; // includes the `:`
  const query = match[2]; // just the text after `:`
  if (!query || query.length < MIN_QUERY_LENGTH) return null;

  const startIndex = beforeCursor.length - full.length;
  return { query, startIndex };
}

export function useEmojiAutocomplete(customEmoji: CustomEmoji[] = []) {
  const [emojiQuery, setEmojiQuery] = React.useState<string | null>(null);
  const [emojiStartIndex, setEmojiStartIndex] = React.useState(0);
  const [emojiSelectedIndex, setEmojiSelectedIndex] = React.useState(0);
  const [suggestions, setSuggestions] = React.useState<EmojiSuggestion[]>([]);
  const [isSearchFailed, setIsSearchFailed] = React.useState(false);
  const suggestionsRef = React.useRef(suggestions);
  suggestionsRef.current = suggestions;
  const selectedIndexRef = React.useRef(emojiSelectedIndex);
  selectedIndexRef.current = emojiSelectedIndex;

  // Keep the latest custom emoji list in a ref so the search effect can read it
  // without re-running on every list-identity change.
  const customEmojiRef = React.useRef(customEmoji);
  customEmojiRef.current = customEmoji;

  const debounceTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const latestValueRef = React.useRef<string>("");
  const latestCursorRef = React.useRef<number>(0);

  React.useEffect(() => {
    return () => {
      if (debounceTimerRef.current !== null) {
        clearTimeout(debounceTimerRef.current);
      }
    };
  }, []);

  React.useEffect(() => {
    if (emojiQuery === null) {
      // Also drops a relay search still waiting out its debounce.
      void emojiAutocompleteSearcher.request("");
      setSuggestions([]);
      setIsSearchFailed(false);
      return;
    }

    let cancelled = false;
    let standard: EmojiSuggestion[] = [];
    let fuzzy: EmojiSuggestion[] = [];
    let relayMatches: EmojiSuggestion[] = [];
    let localReady = false;

    // Custom emoji match by shortcode, separator-insensitive and fuzzy. The
    // pool is the legacy union plus recently used catalog emoji; the rest of
    // the catalog comes from the relay.
    const legacy = customEmojiRef.current;
    const legacyNames = new Set(legacy.map((e) => e.shortcode));
    const pool = [
      ...legacy,
      ...getEmojiUsageSnapshot().filter((e) => !legacyNames.has(e.shortcode)),
    ];
    const customMatches: EmojiSuggestion[] = rankByShortcode(
      emojiQuery,
      pool,
      (e) => e.shortcode,
      UNLIMITED_RESULTS,
    ).map(toCustomSuggestion);

    const publish = (resetSelection: boolean) => {
      const shown = new Set<string>(customMatches.map((e) => e.id));
      // Rank exact/prefix shortcode matches across custom and standard emoji
      // before semantic and weaker matches (for example, `joy` before
      // `bufo_joy`). Keep emoji-mart's name/keyword results ahead of loose
      // substring and subsequence matches.
      const next = rankShortcodeMatchesFirst(
        emojiQuery,
        [
          ...standard,
          ...customMatches,
          ...relayMatches.filter((e) => !shown.has(e.id)),
          ...fuzzy,
        ],
        (emoji) => emoji.id,
      ).slice(0, EMOJI_AUTOCOMPLETE_RESULT_CAP);
      const selectedId = suggestionsRef.current[selectedIndexRef.current]?.id;
      setSuggestions(next);
      setEmojiSelectedIndex(
        resetSelection
          ? 0
          : Math.max(
              0,
              next.findIndex((e) => e.id === selectedId),
            ),
      );
    };

    SearchIndex.search(emojiQuery, {
      caller: "useEmojiAutocomplete",
      maxResults: UNLIMITED_RESULTS,
    })
      .then(
        (
          results: Array<{
            id: string;
            name: string;
            skins: Array<{ native: string }>;
          }> | null,
        ) => {
          if (cancelled) return;
          standard = (results ?? [])
            .map((emoji) => ({
              id: emoji.id,
              name: emoji.name,
              native: emoji.skins[0]?.native ?? "",
            }))
            .filter((e) => e.native !== "");
          // Add fuzzy shortcode matches emoji-mart missed — its token-prefix
          // search can't cross `_` (so `pointup` finds nothing). Skip ids
          // already shown to avoid duplicates.
          const shown = new Set<string>(
            [...customMatches, ...standard].map((e) => e.id),
          );
          fuzzy = fuzzyStandardEmoji(emojiQuery, UNLIMITED_RESULTS, shown).map(
            (e) => ({ id: e.id, name: e.name, native: e.native }),
          );
          localReady = true;
          publish(relayMatches.length === 0);
        },
      )
      .catch(() => {
        if (cancelled) return;
        // emoji-mart failed; still surface custom matches.
        localReady = true;
        publish(true);
      });

    void emojiAutocompleteSearcher.request(emojiQuery).then((outcome) => {
      if (cancelled) return;
      if (outcome.status === "error") {
        console.warn("Community emoji search failed.", outcome.error);
        setIsSearchFailed(true);
        return;
      }
      if (outcome.status !== "ok") return;
      setIsSearchFailed(false);
      relayMatches = outcome.results.map(toCustomSuggestion);
      if (localReady) publish(false);
    });

    return () => {
      cancelled = true;
    };
  }, [emojiQuery]);

  const isEmojiAutocompleteOpen = emojiQuery !== null && suggestions.length > 0;

  const insertEmoji = React.useCallback(
    (suggestion: EmojiSuggestion, selectionEnd: number): AutocompleteEdit => {
      if (debounceTimerRef.current !== null) {
        clearTimeout(debounceTimerRef.current);
        debounceTimerRef.current = null;
      }

      // Custom emoji insert as a selectable atom node (id == shortcode);
      // standard emoji insert their native unicode. Both get a trailing
      // space.
      const isCustom = Boolean(suggestion.url);
      if (isCustom && suggestion.sourceUrl) {
        recordEmojiUse({
          shortcode: suggestion.id,
          url: suggestion.sourceUrl,
        });
      }
      const insertText = isCustom ? " " : `${suggestion.native} `;

      setEmojiQuery(null);
      setEmojiSelectedIndex(0);

      return {
        replaceFromOffset: emojiStartIndex,
        replaceToOffset: selectionEnd,
        insertText,
        ...(isCustom ? { customEmojiShortcode: suggestion.id } : {}),
      };
    },
    [emojiStartIndex],
  );

  const updateEmojiQuery = React.useCallback(
    (value: string, cursorPosition: number) => {
      latestValueRef.current = value;
      latestCursorRef.current = cursorPosition;

      if (debounceTimerRef.current !== null) {
        clearTimeout(debounceTimerRef.current);
        debounceTimerRef.current = null;
      }

      // A cleared draft closes now; see useChannelLinks.
      if (value.length === 0) {
        setEmojiQuery(null);
        return;
      }

      debounceTimerRef.current = setTimeout(() => {
        debounceTimerRef.current = null;
        const result = detectEmojiQuery(
          latestValueRef.current,
          latestCursorRef.current,
        );
        if (result) {
          setEmojiQuery(result.query);
          setEmojiStartIndex(result.startIndex);
        } else {
          setEmojiQuery(null);
        }
      }, EMOJI_DEBOUNCE_MS);
    },
    [],
  );

  const clearEmojis = React.useCallback(() => {
    if (debounceTimerRef.current !== null) {
      clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = null;
    }
    setEmojiQuery(null);
    setEmojiSelectedIndex(0);
    setSuggestions([]);
  }, []);

  const handleEmojiKeyDown = React.useCallback(
    (
      event: React.KeyboardEvent,
    ): { handled: boolean; suggestion?: EmojiSuggestion } => {
      if (!isEmojiAutocompleteOpen) {
        return { handled: false };
      }

      // Forward Tab selects; Shift+Tab deliberately does not. The reverse
      // move stays the browser's, so this overlay can't swallow a keyboard
      // user's way back out (see useMentions for the same split).
      const action = emojiListKeyAction(event, { tabSelects: true });
      if (action === null) return { handled: false };
      event.preventDefault();
      if (action === "next") {
        setEmojiSelectedIndex((current) =>
          current < suggestions.length - 1 ? current + 1 : 0,
        );
        return { handled: true };
      }
      if (action === "prev") {
        setEmojiSelectedIndex((current) =>
          current > 0 ? current - 1 : suggestions.length - 1,
        );
        return { handled: true };
      }
      if (action === "select") {
        return {
          handled: true,
          suggestion: suggestions[emojiSelectedIndex],
        };
      }
      setEmojiQuery(null);
      return { handled: true };
    },
    [isEmojiAutocompleteOpen, emojiSelectedIndex, suggestions],
  );

  return {
    clearEmojis,
    emojiSelectedIndex,
    emojiSuggestions: suggestions,
    handleEmojiKeyDown,
    insertEmoji,
    isEmojiAutocompleteOpen,
    isSearchFailed,
    updateEmojiQuery,
  };
}

export type UseEmojiAutocompleteResult = ReturnType<
  typeof useEmojiAutocomplete
>;
