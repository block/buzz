import * as React from "react";

import { useChannelMembersQuery } from "@/features/channels/hooks";
import {
  extractMentions,
  type MentionRecord,
} from "@/features/dev-mode/lib/mentionRecords";
import { useUserSearchQuery } from "@/features/profile/hooks";
import { detectPrefixQuery } from "@/shared/lib/detectPrefixQuery";
import { normalizePubkey } from "@/shared/lib/pubkey";

const MAX_SUGGESTIONS = 6;

export type MentionSuggestion = MentionRecord & {
  isMember: boolean;
};

/**
 * `@user` autocomplete for a dev-mode composer textarea, mirroring the
 * `#channel` autocomplete's keyboard contract: typing `@` plus a prefix at
 * the caret opens suggestions; ↑/↓ move, Tab/Enter accept (inserting
 * `@Display Name `), Escape dismisses. Channel members rank first, then
 * relay-wide `search_users` results. `extract` maps `@Name`s still present
 * in the text back to pubkeys at send time.
 */
export function useMentionAutocomplete({
  channelId,
  selfPubkey,
  value,
  onChange,
  textareaRef,
}: {
  channelId: string | null;
  selfPubkey: string | null;
  value: string;
  onChange: (value: string) => void;
  textareaRef: React.RefObject<HTMLTextAreaElement | null>;
}) {
  const membersQuery = useChannelMembersQuery(channelId);
  const [cursor, setCursor] = React.useState(0);
  const [selectedIndex, setSelectedIndex] = React.useState(0);
  const [dismissed, setDismissed] = React.useState(false);

  const memberSuggestions = React.useMemo<MentionSuggestion[]>(
    () =>
      (membersQuery.data ?? []).flatMap((member) =>
        member.displayName
          ? [
              {
                pubkey: member.pubkey,
                name: member.displayName,
                isAgent: member.isAgent || member.role === "bot",
                isMember: true,
              },
            ]
          : [],
      ),
    [membersQuery.data],
  );

  // Every relay user seen this composer session, so multi-word names keep
  // the query open across spaces (and suggestions survive the moment a new
  // search key is still loading).
  const [seenRelayUsers, setSeenRelayUsers] = React.useState<
    ReadonlyMap<string, MentionSuggestion>
  >(new Map());

  const knownNamesLower = React.useMemo(
    () => [
      ...memberSuggestions.map((suggestion) => suggestion.name.toLowerCase()),
      ...[...seenRelayUsers.values()].map((suggestion) =>
        suggestion.name.toLowerCase(),
      ),
    ],
    [memberSuggestions, seenRelayUsers],
  );

  const detection = React.useMemo(
    () => detectPrefixQuery("@", value, cursor, knownNamesLower),
    [cursor, knownNamesLower, value],
  );

  const searchQuery = useUserSearchQuery(detection?.query ?? "", {
    enabled: detection !== null,
    limit: 8,
  });

  const searchResults = searchQuery.data;
  React.useEffect(() => {
    if (!searchResults?.length) return;
    setSeenRelayUsers((current) => {
      let next: Map<string, MentionSuggestion> | null = null;
      for (const user of searchResults) {
        const name = user.displayName?.trim();
        if (!name) continue;
        const key = normalizePubkey(user.pubkey);
        if (current.has(key)) continue;
        if (!next) next = new Map(current);
        next.set(key, {
          pubkey: user.pubkey,
          name,
          isAgent: user.isAgent,
          isMember: false,
        });
      }
      return next ?? current;
    });
  }, [searchResults]);

  const suggestions = React.useMemo<MentionSuggestion[]>(() => {
    if (!detection) return [];
    const candidates: MentionSuggestion[] = [
      ...memberSuggestions,
      ...(searchResults ?? []).flatMap((user) => {
        const name = user.displayName?.trim();
        return name
          ? [
              {
                pubkey: user.pubkey,
                name,
                isAgent: user.isAgent,
                isMember: false,
              },
            ]
          : [];
      }),
      ...seenRelayUsers.values(),
    ];
    const needle = detection.query.toLowerCase();
    const seen = new Set<string>();
    const starts: MentionSuggestion[] = [];
    const contains: MentionSuggestion[] = [];
    for (const candidate of candidates) {
      const key = normalizePubkey(candidate.pubkey);
      if (seen.has(key)) continue;
      if (selfPubkey && key === normalizePubkey(selfPubkey)) continue;
      const name = candidate.name.toLowerCase();
      if (name.startsWith(needle)) {
        starts.push(candidate);
      } else if (name.includes(needle)) {
        contains.push(candidate);
      } else {
        continue;
      }
      seen.add(key);
    }
    return [...starts, ...contains].slice(0, MAX_SUGGESTIONS);
  }, [detection, memberSuggestions, searchResults, seenRelayUsers, selfPubkey]);

  const open = !dismissed && suggestions.length > 0;

  // A fresh query (typing/caret movement) clears the selection and any
  // Escape dismissal from the previous query.
  const queryKey = detection
    ? `${detection.startIndex}:${detection.query}`
    : null;
  const previousQueryKeyRef = React.useRef(queryKey);
  if (previousQueryKeyRef.current !== queryKey) {
    previousQueryKeyRef.current = queryKey;
    if (selectedIndex !== 0) setSelectedIndex(0);
    if (dismissed) setDismissed(false);
  }

  const syncCursor = React.useCallback((target: HTMLTextAreaElement) => {
    setCursor(target.selectionStart ?? target.value.length);
  }, []);

  // Accepted suggestions, so `extract` can map typed `@Name`s back to
  // pubkeys — relay users aren't channel members yet.
  const acceptedRef = React.useRef(new Map<string, MentionRecord>());

  const accept = React.useCallback(
    (suggestion: MentionSuggestion) => {
      if (!detection) return;
      const before = value.slice(0, detection.startIndex);
      const inserted = `@${suggestion.name} `;
      const next = before + inserted + value.slice(cursor);
      onChange(next);
      acceptedRef.current.set(normalizePubkey(suggestion.pubkey), {
        name: suggestion.name,
        pubkey: suggestion.pubkey,
        isAgent: suggestion.isAgent,
      });
      const caret = before.length + inserted.length;
      setCursor(caret);
      requestAnimationFrame(() => {
        textareaRef.current?.setSelectionRange(caret, caret);
      });
    },
    [cursor, detection, onChange, textareaRef, value],
  );

  const handleKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLTextAreaElement>): boolean => {
      if (!open) return false;
      // Modified keys belong to composer shortcuts (e.g. ⌥↑ channel step).
      if (event.altKey || event.metaKey || event.ctrlKey) return false;
      if (event.key === "ArrowUp" || event.key === "ArrowDown") {
        event.preventDefault();
        const delta = event.key === "ArrowUp" ? -1 : 1;
        setSelectedIndex(
          (current) =>
            (current + delta + suggestions.length) % suggestions.length,
        );
        return true;
      }
      if (event.key === "Tab" || event.key === "Enter") {
        event.preventDefault();
        accept(suggestions[selectedIndex] ?? suggestions[0]);
        return true;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setDismissed(true);
        return true;
      }
      return false;
    },
    [accept, open, selectedIndex, suggestions],
  );

  /** Mentions still present in the text: accepted users + channel members. */
  const extract = React.useCallback(
    (text: string): MentionRecord[] =>
      extractMentions(text, [
        ...acceptedRef.current.values(),
        ...memberSuggestions,
      ]),
    [memberSuggestions],
  );

  return {
    open,
    suggestions,
    selectedIndex,
    accept,
    handleKeyDown,
    syncCursor,
    extract,
  };
}
