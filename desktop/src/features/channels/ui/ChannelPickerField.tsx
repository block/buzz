import { Check, Search } from "lucide-react";
import * as React from "react";

import { filterChannelsByQuery } from "@/features/channels/lib/channelPickerOrdering";
import type { Channel } from "@/shared/api/types";

type ChannelPickerFieldProps = {
  /** Candidate channels, in display order (see `sortChannelsMembersFirst`). */
  channels: Channel[];
  disabled?: boolean;
  /** Id for the search input, so a `<label htmlFor>` can target it. */
  inputId: string;
  onChange: (channelId: string) => void;
  /** Id of the currently selected channel. */
  value: string;
};

/**
 * Searchable channel list for dialog forms. Replaces the plain `<select>`
 * that listed every community channel in one dropdown — communities with
 * many channels made that unusable. Fuzzy-matches with the channel
 * browser's scorer and keeps the caller's (members-first) ordering.
 *
 * Selection semantics: this field owns keeping `value` consistent with the
 * visible list. It selects the first visible channel when `value` is empty
 * or filtered out, and clears `value` when the query matches nothing — so
 * a hidden channel can never remain the submit target.
 */
export function ChannelPickerField({
  channels,
  disabled = false,
  inputId,
  onChange,
  value,
}: ChannelPickerFieldProps) {
  const [query, setQuery] = React.useState("");
  const listRef = React.useRef<HTMLDivElement>(null);

  const visibleChannels = React.useMemo(
    () => filterChannelsByQuery(channels, query),
    [channels, query],
  );

  function handleQueryChange(nextQuery: string) {
    const nextVisibleChannels = filterChannelsByQuery(channels, nextQuery);
    setQuery(nextQuery);

    // Reconcile the parent value in the same input event. Deferring this to
    // the effect below leaves the previous channel actionable for one render,
    // so a no-match query could submit a hidden target before the effect runs.
    if (nextVisibleChannels.length === 0) {
      if (value !== "") {
        onChange("");
      }
    } else if (!nextVisibleChannels.some((channel) => channel.id === value)) {
      onChange(nextVisibleChannels[0].id);
    }
  }

  // Keep the selection visible: when the query filters out the selected
  // channel, move it to the best match — the `<select>` this replaces
  // could never hold an invisible selection. When nothing matches, clear
  // the selection so the parent form cannot submit to a hidden channel.
  // Once the selection is in the list, keep its row scrolled into view
  // for keyboard navigation.
  React.useEffect(() => {
    if (visibleChannels.length === 0) {
      if (value !== "") {
        onChange("");
      }
      return;
    }
    if (!visibleChannels.some((channel) => channel.id === value)) {
      onChange(visibleChannels[0].id);
      return;
    }
    listRef.current
      ?.querySelector('[aria-selected="true"]')
      ?.scrollIntoView({ block: "nearest" });
  }, [onChange, value, visibleChannels]);

  function moveSelection(delta: -1 | 1) {
    if (visibleChannels.length === 0) {
      return;
    }
    const currentIndex = visibleChannels.findIndex(
      (channel) => channel.id === value,
    );
    const nextIndex =
      currentIndex === -1
        ? delta === 1
          ? 0
          : visibleChannels.length - 1
        : Math.min(
            Math.max(currentIndex + delta, 0),
            visibleChannels.length - 1,
          );
    onChange(visibleChannels[nextIndex].id);
  }

  const listId = `${inputId}-listbox`;
  const optionId = (channelId: string) => `${inputId}-option-${channelId}`;
  const selectedIsVisible = visibleChannels.some(
    (channel) => channel.id === value,
  );

  return (
    <div className="space-y-1.5">
      <div className="relative">
        <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground/55" />
        <input
          aria-activedescendant={
            selectedIsVisible ? optionId(value) : undefined
          }
          aria-autocomplete="list"
          aria-controls={listId}
          aria-expanded={true}
          autoCapitalize="none"
          autoCorrect="off"
          className="flex h-9 w-full rounded-md border border-input bg-background py-2 pl-9 pr-3 text-sm shadow-xs placeholder:text-muted-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
          disabled={disabled}
          id={inputId}
          onChange={(event) => handleQueryChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              moveSelection(1);
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              moveSelection(-1);
            }
          }}
          placeholder="Search channels"
          role="combobox"
          spellCheck={false}
          type="text"
          value={query}
        />
      </div>
      <div
        aria-label="Channels"
        className="max-h-52 divide-y divide-border/40 overflow-y-auto rounded-md border border-input bg-background shadow-xs"
        id={listId}
        ref={listRef}
        role="listbox"
      >
        {visibleChannels.length === 0 ? (
          <p className="px-3 py-6 text-center text-sm text-muted-foreground">
            {channels.length === 0
              ? "No channels available"
              : `No channels match “${query.trim()}”`}
          </p>
        ) : (
          visibleChannels.map((channel) => {
            const isSelected = channel.id === value;
            return (
              <button
                aria-selected={isSelected}
                className={
                  isSelected
                    ? "flex w-full items-center gap-2 bg-muted/60 px-3 py-2 text-left transition-colors duration-150 ease-out focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
                    : "flex w-full items-center gap-2 px-3 py-2 text-left transition-colors duration-150 ease-out hover:bg-muted/40 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
                }
                disabled={disabled}
                id={optionId(channel.id)}
                key={channel.id}
                onClick={() => onChange(channel.id)}
                onMouseDown={(event) => event.preventDefault()}
                role="option"
                tabIndex={-1}
                type="button"
              >
                <span className="min-w-0 flex-1 truncate text-sm">
                  <span className="text-muted-foreground"># </span>
                  <span className="font-medium">{channel.name}</span>
                  <span className="ml-2 text-xs text-muted-foreground">
                    {channel.visibility}
                    {channel.isMember ? " · joined" : ""}
                  </span>
                </span>
                {isSelected ? (
                  <Check className="h-4 w-4 shrink-0 text-primary" />
                ) : null}
              </button>
            );
          })
        )}
      </div>
    </div>
  );
}
