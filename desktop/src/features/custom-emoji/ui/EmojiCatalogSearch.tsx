import * as React from "react";

import { emojiPickerSearcher } from "@/features/custom-emoji/emojiCatalog";
import {
  EMOJI_PICKER_RESULT_CAP,
  EMOJI_SEARCH_MIN_LENGTH,
  type PrefixSearcher,
} from "@/features/custom-emoji/emojiCatalogSource";
import { emojiListKeyAction } from "@/features/custom-emoji/emojiListKeys";
import { cn } from "@/shared/lib/cn";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";
import { Input } from "@/shared/ui/input";

type SearchState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ok"; results: CustomEmoji[] }
  | { kind: "error" };

type EmojiCatalogSearchProps = {
  onSelect: (emoji: CustomEmoji) => void;
  searcher?: PrefixSearcher;
};

/**
 * Search box and result list for the relay emoji catalog, shown above the
 * emoji-mart picker (which only takes a static list). Focus stays in the input;
 * the list is a combobox listbox driven by `aria-activedescendant`.
 */
export function EmojiCatalogSearch({
  onSelect,
  searcher = emojiPickerSearcher,
}: EmojiCatalogSearchProps) {
  const listId = React.useId();
  const [query, setQuery] = React.useState("");
  const [retry, setRetry] = React.useState(0);
  const [state, setState] = React.useState<SearchState>({ kind: "idle" });
  const [activeIndex, setActiveIndex] = React.useState(0);

  // biome-ignore lint/correctness/useExhaustiveDependencies: `retry` only forces a new request for the same query.
  React.useEffect(() => {
    let cancelled = false;
    setActiveIndex(0);
    if (query.trim().length < EMOJI_SEARCH_MIN_LENGTH) {
      void searcher.request("");
      setState({ kind: "idle" });
      return;
    }
    setState({ kind: "loading" });
    void searcher.request(query).then((outcome) => {
      if (cancelled) return;
      if (outcome.status === "ok") {
        setState({ kind: "ok", results: outcome.results });
      } else if (outcome.status === "error") {
        console.warn("Community emoji search failed.", outcome.error);
        setState({ kind: "error" });
      } else if (outcome.status === "short") {
        setState({ kind: "idle" });
      }
    });
    return () => {
      cancelled = true;
    };
  }, [query, retry, searcher]);

  const results = state.kind === "ok" ? state.results : [];
  const optionId = (index: number) => `${listId}-${index}`;

  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    const action = emojiListKeyAction(event, { tabSelects: false });
    if (action === null) return;
    if (action === "close") {
      if (query === "") return;
      event.preventDefault();
      event.stopPropagation();
      setQuery("");
      return;
    }
    if (results.length === 0) return;
    event.preventDefault();
    if (action === "next") {
      setActiveIndex((index) => (index + 1) % results.length);
    } else if (action === "prev") {
      setActiveIndex((index) => (index - 1 + results.length) % results.length);
    } else {
      const chosen = results[activeIndex];
      if (chosen) onSelect(chosen);
    }
  };

  let status: React.ReactNode = null;
  if (state.kind === "loading") {
    status = "Searching…";
  } else if (state.kind === "error") {
    status = (
      <>
        Community emoji search failed.{" "}
        <button
          className="underline underline-offset-2"
          onClick={() => setRetry((count) => count + 1)}
          type="button"
        >
          Retry
        </button>
      </>
    );
  } else if (state.kind === "ok") {
    if (results.length === 0) {
      status = "No community emoji match.";
    } else if (results.length >= EMOJI_PICKER_RESULT_CAP) {
      status = `Showing the first ${results.length}. Keep typing to narrow.`;
    } else {
      status = `${results.length} ${results.length === 1 ? "result" : "results"}`;
    }
  }

  return (
    <div className="border-b border-border/60 bg-muted p-2">
      <Input
        aria-activedescendant={
          results.length > 0 ? optionId(activeIndex) : undefined
        }
        aria-autocomplete="list"
        aria-controls={results.length > 0 ? listId : undefined}
        aria-expanded={results.length > 0}
        aria-label="Search all community emoji"
        className="h-8"
        data-testid="emoji-catalog-search"
        onChange={(event) => setQuery(event.target.value)}
        onKeyDown={handleKeyDown}
        placeholder="Search all community emoji"
        role="combobox"
        type="text"
        value={query}
      />
      <p
        aria-live="polite"
        className="min-h-4 px-1 pt-1 text-2xs text-muted-foreground"
        data-testid="emoji-catalog-status"
        role="status"
      >
        {status}
      </p>
      {results.length > 0 ? (
        <div
          aria-label="Community emoji results"
          className="max-h-40 overflow-y-auto"
          id={listId}
          role="listbox"
        >
          {results.map((emoji, index) => (
            <div
              aria-label={`:${emoji.shortcode}:`}
              aria-selected={index === activeIndex}
              className={cn(
                "flex cursor-pointer items-center gap-2 rounded-lg px-2 py-1 text-sm",
                index === activeIndex
                  ? "bg-accent text-accent-foreground"
                  : "hover:bg-accent/50",
              )}
              data-testid="emoji-catalog-result"
              id={optionId(index)}
              key={emoji.shortcode}
              onMouseDown={(event) => {
                event.preventDefault();
                onSelect(emoji);
              }}
              role="option"
              tabIndex={-1}
            >
              <img
                alt=""
                className="h-5 w-5 object-contain"
                draggable={false}
                src={rewriteRelayUrl(emoji.url)}
              />
              <span aria-hidden="true" className="truncate">
                :{emoji.shortcode}:
              </span>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}
