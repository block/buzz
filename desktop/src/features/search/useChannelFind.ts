import * as React from "react";

import { useSearchMessagesQuery } from "@/features/search/hooks";
import type { TimelineMessage } from "@/features/messages/types";
import { cacheSearchHitEvent } from "@/app/navigation/searchHitEventCache";
import type { RelayEvent, SearchHit } from "@/shared/api/types";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

const MIN_QUERY_LENGTH = 2;
const DEBOUNCE_MS = 300;

type UseChannelFindOptions = {
  channelId: string | null;
  messages: TimelineMessage[];
  onSearchHit?: (hit: SearchHit) => void;
};

export function useChannelFind({
  channelId,
  messages,
  onSearchHit,
}: UseChannelFindOptions) {
  const [isOpen, setIsOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [debouncedQuery, setDebouncedQuery] = React.useState("");
  const [activeIndex, setActiveIndex] = React.useState(0);

  const reset = React.useCallback(() => {
    setIsOpen(false);
    setQuery("");
    setDebouncedQuery("");
    setActiveIndex(0);
  }, []);

  // Debounce the query for relay search.
  React.useEffect(() => {
    const trimmed = query.trim();
    if (trimmed.length < MIN_QUERY_LENGTH) {
      setDebouncedQuery("");
      return;
    }

    const timeout = window.setTimeout(() => {
      setDebouncedQuery(trimmed);
    }, DEBOUNCE_MS);

    return () => window.clearTimeout(timeout);
  }, [query]);

  // Client-side search: instant matches against loaded messages.
  const clientMatchIds = React.useMemo<string[]>(() => {
    const trimmed = query.trim().toLowerCase();
    if (trimmed.length < MIN_QUERY_LENGTH) {
      return [];
    }

    const found: string[] = [];
    for (const message of messages) {
      if (message.body.toLowerCase().includes(trimmed)) {
        found.push(message.id);
      }
    }

    return found;
  }, [messages, query]);

  // Relay-backed search: full history via Postgres FTS.
  const relaySearch = useSearchMessagesQuery(debouncedQuery, {
    channelId: channelId ?? undefined,
    enabled: isOpen && debouncedQuery.length >= MIN_QUERY_LENGTH,
    limit: 100,
  });

  // Merge: start with client-side matches, then supplement with relay hits.
  // Relay hits may refer to older messages outside the initial cold window;
  // keep them in the match list and ask the route-target splice path to load
  // the active hit so the DOM-based timeline scroll can land it.
  const matchedIds = React.useMemo<string[]>(() => {
    const merged = [...clientMatchIds];
    const seen = new Set(merged);

    if (relaySearch.data?.hits) {
      for (const hit of relaySearch.data.hits) {
        if (!seen.has(hit.eventId)) {
          merged.push(hit.eventId);
          seen.add(hit.eventId);
        }
      }
    }

    return merged;
  }, [clientMatchIds, relaySearch.data?.hits]);

  // Clamp active index when results change.
  React.useEffect(() => {
    setActiveIndex((current) => {
      if (matchedIds.length === 0) return 0;
      return current >= matchedIds.length ? 0 : current;
    });
  }, [matchedIds.length]);

  const activeMatch =
    matchedIds.length > 0 ? { messageId: matchedIds[activeIndex] } : null;

  const relayHitById = React.useMemo(() => {
    const hits = new Map<string, SearchHit>();
    for (const hit of relaySearch.data?.hits ?? []) {
      hits.set(hit.eventId, hit);
    }
    return hits;
  }, [relaySearch.data?.hits]);

  React.useEffect(() => {
    if (!activeMatch) return;
    const hit = relayHitById.get(activeMatch.messageId);
    if (hit) onSearchHit?.(hit);
  }, [activeMatch, onSearchHit, relayHitById]);

  const matchingMessageIds = React.useMemo(() => {
    return new Set(matchedIds);
  }, [matchedIds]);

  const close = React.useCallback(() => {
    reset();
  }, [reset]);

  const goToNext = React.useCallback(() => {
    if (matchedIds.length === 0) return;
    setActiveIndex((current) => (current + 1) % matchedIds.length);
  }, [matchedIds.length]);

  const goToPrevious = React.useCallback(() => {
    if (matchedIds.length === 0) return;
    setActiveIndex((current) =>
      current === 0 ? matchedIds.length - 1 : current - 1,
    );
  }, [matchedIds.length]);

  // Register platform-standard find shortcut (⌘F on macOS, Ctrl+F elsewhere).
  React.useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (
        hasPrimaryShortcutModifier(event) &&
        !event.altKey &&
        !event.shiftKey &&
        event.key.toLowerCase() === "f"
      ) {
        event.preventDefault();
        setIsOpen(true);
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  // Close find bar when switching channels.
  const prevChannelIdRef = React.useRef(channelId);
  React.useEffect(() => {
    if (prevChannelIdRef.current !== channelId) {
      prevChannelIdRef.current = channelId;
      reset();
    }
  }, [channelId, reset]);

  return React.useMemo(
    () => ({
      activeIndex,
      activeMatch,
      close,
      goToNext,
      goToPrevious,
      isOpen,
      matchCount: matchedIds.length,
      matchingMessageIds,
      query,
      setQuery,
    }),
    [
      activeIndex,
      activeMatch,
      close,
      goToNext,
      goToPrevious,
      isOpen,
      matchedIds.length,
      matchingMessageIds,
      query,
    ],
  );
}

/**
 * Ownership of the "find target" events: search hits outside the loaded
 * window are cached and spliced into the timeline, and cleared when the
 * channel changes. Split from useChannelFind because the spliced events
 * feed the very message list the find hook searches.
 */
export function useFindTargetEvents(channelId: string | null) {
  const [findTargetEvents, setFindTargetEvents] = React.useState<RelayEvent[]>(
    [],
  );
  // biome-ignore lint/correctness/useExhaustiveDependencies: clear spliced find results exactly when the active channel changes.
  React.useEffect(() => {
    setFindTargetEvents([]);
  }, [channelId]);
  const handleFindSearchHit = React.useCallback((hit: SearchHit) => {
    const event = cacheSearchHitEvent(hit);
    setFindTargetEvents((currentEvents) =>
      currentEvents.some((currentEvent) => currentEvent.id === event.id)
        ? currentEvents
        : [...currentEvents, event],
    );
  }, []);
  return { findTargetEvents, handleFindSearchHit };
}
