import * as React from "react";

import { useSearchMessagesQuery } from "@/features/search/hooks";
import type { TimelineMessage } from "@/features/messages/types";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

const MIN_QUERY_LENGTH = 2;
const DEBOUNCE_MS = 300;

type UseChannelFindOptions = {
  channelId: string | null;
  messages: TimelineMessage[];
};

export function useChannelFind({ channelId, messages }: UseChannelFindOptions) {
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

  // Relay-backed search: full history via Typesense.
  const relaySearch = useSearchMessagesQuery(debouncedQuery, {
    channelId: channelId ?? undefined,
    enabled: isOpen && debouncedQuery.length >= MIN_QUERY_LENGTH,
    limit: 100,
  });

  // Merge: start with client-side matches, then supplement with relay hits
  // that are loaded in the timeline but were missed by exact substring match
  // (e.g. Typesense stemming). Only loaded messages are kept so the match
  // count stays accurate relative to what's visible on screen.
  const loadedMessageIds = React.useMemo(
    () => new Set(messages.map((m) => m.id)),
    [messages],
  );

  const matchedIds = React.useMemo<string[]>(() => {
    const merged = [...clientMatchIds];
    const seen = new Set(merged);

    if (relaySearch.data?.hits) {
      for (const hit of relaySearch.data.hits) {
        if (!seen.has(hit.eventId) && loadedMessageIds.has(hit.eventId)) {
          merged.push(hit.eventId);
          seen.add(hit.eventId);
        }
      }
    }

    return merged;
  }, [clientMatchIds, relaySearch.data?.hits, loadedMessageIds]);

  // Clamp active index when results change.
  React.useEffect(() => {
    setActiveIndex((current) => {
      if (matchedIds.length === 0) return 0;
      return current >= matchedIds.length ? 0 : current;
    });
  }, [matchedIds.length]);

  const activeMatch =
    matchedIds.length > 0 ? { messageId: matchedIds[activeIndex] } : null;

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

  return {
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
  };
}
