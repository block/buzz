import * as React from "react";

import { init, SearchIndex } from "emoji-mart";
import data from "@emoji-mart/data";

import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import type { AutocompleteEdit } from "./useRichTextEditor";

export type EmojiSuggestion = {
  id: string;
  name: string;
  native: string;
  /** Set for custom (image) emoji; absent for standard unicode emoji. */
  url?: string;
};

const EMOJI_DEBOUNCE_MS = 120;
const MIN_QUERY_LENGTH = 2;
const MAX_RESULTS = 8;

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
      setSuggestions([]);
      return;
    }

    let cancelled = false;

    // Custom emoji match by shortcode prefix/substring (case-insensitive).
    const q = emojiQuery.toLowerCase();
    const customMatches: EmojiSuggestion[] = customEmojiRef.current
      .filter((e) => e.shortcode.toLowerCase().includes(q))
      .slice(0, MAX_RESULTS)
      .map((e) => ({
        id: e.shortcode,
        name: e.shortcode,
        native: "",
        url: rewriteRelayUrl(e.url),
      }));

    SearchIndex.search(emojiQuery)
      .then(
        (
          results: Array<{
            id: string;
            name: string;
            skins: Array<{ native: string }>;
          }> | null,
        ) => {
          if (cancelled) return;
          const standard: EmojiSuggestion[] = (results ?? [])
            .map((emoji) => ({
              id: emoji.id,
              name: emoji.name,
              native: emoji.skins[0]?.native ?? "",
            }))
            .filter((e) => e.native !== "");
          // Custom emoji first (workspace-specific), then standard, capped.
          const merged = [...customMatches, ...standard].slice(0, MAX_RESULTS);
          setSuggestions(merged);
          setEmojiSelectedIndex(0);
        },
      )
      .catch(() => {
        if (cancelled) return;
        // emoji-mart failed; still surface custom matches.
        setSuggestions(customMatches);
        setEmojiSelectedIndex(0);
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

      if (event.key === "ArrowDown") {
        event.preventDefault();
        setEmojiSelectedIndex((current) =>
          current < suggestions.length - 1 ? current + 1 : 0,
        );
        return { handled: true };
      }

      if (event.key === "ArrowUp") {
        event.preventDefault();
        setEmojiSelectedIndex((current) =>
          current > 0 ? current - 1 : suggestions.length - 1,
        );
        return { handled: true };
      }

      if (
        event.key === "Tab" ||
        (event.key === "Enter" &&
          !event.ctrlKey &&
          !event.metaKey &&
          !event.altKey &&
          !event.shiftKey)
      ) {
        event.preventDefault();
        return {
          handled: true,
          suggestion: suggestions[emojiSelectedIndex],
        };
      }

      if (event.key === "Escape") {
        event.preventDefault();
        setEmojiQuery(null);
        return { handled: true };
      }

      return { handled: false };
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
    updateEmojiQuery,
  };
}

export type UseEmojiAutocompleteResult = ReturnType<
  typeof useEmojiAutocomplete
>;
