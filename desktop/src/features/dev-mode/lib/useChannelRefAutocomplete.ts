import * as React from "react";

import {
  type ChannelRef,
  useChannelRefs,
} from "@/features/dev-mode/lib/channelRefs";
import { detectPrefixQuery } from "@/shared/lib/detectPrefixQuery";

const MAX_SUGGESTIONS = 6;

/**
 * `#channel` autocomplete for a dev-mode composer textarea. Typing `#` plus
 * a prefix at the caret opens suggestions; ↑/↓ move, Tab/Enter accept
 * (inserting `#channel-name `), Escape dismisses. `handleKeyDown` returns
 * true when it consumed the key, so callers try it before their own
 * bindings (Tab normally cycles modes, Enter normally sends).
 */
export function useChannelRefAutocomplete({
  value,
  onChange,
  textareaRef,
}: {
  value: string;
  onChange: (value: string) => void;
  textareaRef: React.RefObject<HTMLTextAreaElement | null>;
}) {
  const { channels } = useChannelRefs();
  const [cursor, setCursor] = React.useState(0);
  const [selectedIndex, setSelectedIndex] = React.useState(0);
  const [dismissed, setDismissed] = React.useState(false);

  const knownNamesLower = React.useMemo(
    () => channels.map((channel) => channel.name.toLowerCase()),
    [channels],
  );

  const detection = React.useMemo(
    () => detectPrefixQuery("#", value, cursor, knownNamesLower),
    [cursor, knownNamesLower, value],
  );

  const suggestions = React.useMemo<ChannelRef[]>(() => {
    if (!detection) return [];
    const needle = detection.query.toLowerCase();
    const starts: ChannelRef[] = [];
    const contains: ChannelRef[] = [];
    for (const channel of channels) {
      const name = channel.name.toLowerCase();
      if (name.startsWith(needle)) {
        starts.push(channel);
      } else if (name.includes(needle)) {
        contains.push(channel);
      }
    }
    return [...starts, ...contains].slice(0, MAX_SUGGESTIONS);
  }, [channels, detection]);

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

  const accept = React.useCallback(
    (suggestion: ChannelRef) => {
      if (!detection) return;
      const before = value.slice(0, detection.startIndex);
      const inserted = `#${suggestion.name} `;
      const next = before + inserted + value.slice(cursor);
      onChange(next);
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

  return {
    open,
    suggestions,
    selectedIndex,
    query: detection?.query ?? "",
    accept,
    handleKeyDown,
    syncCursor,
  };
}
