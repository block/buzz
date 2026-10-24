import {
  Asterisk,
  Check,
  ChevronDown,
  Hash,
  Lock,
  MessageSquareMore,
  Search,
} from "lucide-react";
import * as React from "react";

import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveChannelDisplayLabel } from "@/features/sidebar/lib/channelLabels";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { PortalledScrollArea } from "@/shared/ui/PortalledScrollArea";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

function ChannelPrivacyIcon({ channel }: { channel: Channel }) {
  const Icon =
    channel.channelType === "dm"
      ? MessageSquareMore
      : channel.visibility === "private"
        ? Lock
        : Hash;

  return (
    <Icon aria-hidden className="h-5 w-5 shrink-0 text-muted-foreground/60" />
  );
}

type ChannelComboboxProps = {
  allowEmpty?: boolean;
  ariaLabel?: string;
  channels: Channel[];
  defaultOpen?: boolean;
  disabled?: boolean;
  emptyLabel?: string;
  id?: string;
  isChannelDisabled?: (channel: Channel) => boolean;
  onAutoOpen?: () => void;
  onChange: (value: string) => void;
  readOnly?: boolean;
  readOnlyTooltip?: string;
  required?: boolean;
  variant?: "header" | "field";
  value: string;
};

export function ChannelCombobox({
  allowEmpty = false,
  ariaLabel = "Channel",
  channels,
  defaultOpen = false,
  disabled,
  emptyLabel = "Choose a channel",
  id,
  isChannelDisabled,
  onAutoOpen,
  onChange,
  readOnly = false,
  readOnlyTooltip = "The channel can't be changed after a workflow is created.",
  required = false,
  variant = "header",
  value,
}: ChannelComboboxProps) {
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [highlightedIndex, setHighlightedIndex] = React.useState(0);
  const autoOpenHandledRef = React.useRef(false);

  React.useEffect(() => {
    if (!defaultOpen || autoOpenHandledRef.current) return;

    // Let the pointer interaction that mounted the containing dialog finish
    // before installing Radix's outside-interaction listeners.
    const frame = window.requestAnimationFrame(() => {
      autoOpenHandledRef.current = true;
      setOpen(true);
      onAutoOpen?.();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [defaultOpen, onAutoOpen]);

  const listboxId = `${id ?? "channel"}-listbox`;
  const selected = channels.find((c) => c.id === value);
  const currentPubkey = useIdentityQuery().data?.pubkey;
  const dmParticipantPubkeys = React.useMemo(() => {
    const visibleDmChannels = open
      ? channels.filter((channel) => channel.channelType === "dm")
      : selected?.channelType === "dm"
        ? [selected]
        : [];

    return visibleDmChannels.flatMap((channel) =>
      channel.participantPubkeys.filter(
        (pubkey) => pubkey.toLowerCase() !== currentPubkey?.toLowerCase(),
      ),
    );
  }, [channels, currentPubkey, open, selected]);
  const dmProfiles = useUsersBatchQuery(dmParticipantPubkeys, {
    enabled: dmParticipantPubkeys.length > 0,
  }).data?.profiles;
  const channelLabels = React.useMemo(
    () =>
      new Map(
        channels.map((channel) => [
          channel.id,
          resolveChannelDisplayLabel(channel, currentPubkey, dmProfiles),
        ]),
      ),
    [channels, currentPubkey, dmProfiles],
  );

  const filtered = React.useMemo(() => {
    if (!query) return channels;
    const q = query.toLowerCase();
    return channels.filter(
      (c) =>
        (channelLabels.get(c.id) ?? c.name).toLowerCase().includes(q) ||
        c.channelType?.toLowerCase().includes(q) ||
        c.id.toLowerCase().includes(q),
    );
  }, [channelLabels, channels, query]);
  const selectable = React.useMemo(
    () => filtered.filter((channel) => !isChannelDisabled?.(channel)),
    [filtered, isChannelDisabled],
  );
  const highlightedChannel = selectable[highlightedIndex];
  const highlightedOptionId = highlightedChannel
    ? `${listboxId}-option-${highlightedChannel.id}`
    : undefined;

  function handleOpenChange(next: boolean) {
    setOpen(next);
    if (!next) {
      setQuery("");
      setHighlightedIndex(0);
    }
  }

  function selectChannel(channelId: string) {
    onChange(channelId);
    handleOpenChange(false);
  }

  function handleKeyDown(e: React.KeyboardEvent) {
    if (selectable.length === 0) return;

    switch (e.key) {
      case "ArrowDown": {
        e.preventDefault();
        setHighlightedIndex((i) => (i + 1) % selectable.length);
        break;
      }
      case "ArrowUp": {
        e.preventDefault();
        setHighlightedIndex(
          (i) => (i - 1 + selectable.length) % selectable.length,
        );
        break;
      }
      case "Enter": {
        e.preventDefault();
        const target = selectable[highlightedIndex];
        if (target) selectChannel(target.id);
        break;
      }
      case "Escape": {
        e.preventDefault();
        handleOpenChange(false);
        break;
      }
    }
  }

  const selectedLabel = selected
    ? (channelLabels.get(selected.id) ?? selected.name)
    : value
      ? "Unavailable channel"
      : emptyLabel;

  if (readOnly) {
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            aria-disabled="true"
            aria-label={`${ariaLabel}: ${selectedLabel}. Read only.`}
            className={cn(
              "flex w-full cursor-default items-center rounded-lg px-3 py-2 text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring",
              variant === "header" &&
                "justify-center border-0 bg-transparent text-lg font-semibold",
              variant === "field" && "text-sm font-normal",
            )}
            id={id}
            type="button"
          >
            <span className="flex min-w-0 items-center gap-2">
              {selected ? <ChannelPrivacyIcon channel={selected} /> : null}
              <span className="truncate">{selectedLabel}</span>
            </span>
          </button>
        </TooltipTrigger>
        <TooltipContent side="top">{readOnlyTooltip}</TooltipContent>
      </Tooltip>
    );
  }

  return (
    <Popover onOpenChange={handleOpenChange} open={open}>
      <PopoverTrigger asChild>
        <button
          aria-controls={listboxId}
          aria-expanded={open}
          aria-haspopup="listbox"
          aria-label={ariaLabel}
          aria-required={required}
          className={cn(
            "group flex w-full items-center rounded-lg px-3 py-2 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
            variant === "header" &&
              "justify-center border-0 bg-transparent text-lg font-semibold",
            variant === "field" &&
              "justify-between border border-input/40 bg-background text-sm font-normal",
            !selected && "text-muted-foreground",
          )}
          disabled={disabled}
          id={id}
          role="combobox"
          type="button"
        >
          <span
            className={cn(
              "flex min-w-0 items-center gap-2",
              variant === "header" && "justify-center",
            )}
          >
            {selected ? (
              <ChannelPrivacyIcon channel={selected} />
            ) : required && !value ? (
              <Asterisk aria-hidden className="h-5 w-5 shrink-0 text-primary" />
            ) : null}
            <span className="truncate">{selectedLabel}</span>
          </span>
          <ChevronDown className="ml-1 h-5 w-5 shrink-0 text-muted-foreground opacity-50 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-(--radix-popover-trigger-width) p-0"
        portalled={variant === "field"}
      >
        <div className="flex items-center gap-2 border-b border-border px-3 py-2">
          <Search className="h-4 w-4 shrink-0 text-muted-foreground" />
          <input
            aria-activedescendant={highlightedOptionId}
            aria-autocomplete="list"
            aria-controls={listboxId}
            aria-expanded={open}
            aria-label={`Search ${ariaLabel.toLowerCase()}s`}
            autoCapitalize="none"
            autoComplete="off"
            autoCorrect="off"
            ref={(el) => el?.focus()}
            className="flex-1 bg-transparent text-sm outline-hidden placeholder:text-muted-foreground"
            onChange={(e) => {
              setQuery(e.target.value);
              setHighlightedIndex(0);
            }}
            onKeyDown={handleKeyDown}
            placeholder="Search channels..."
            role="combobox"
            spellCheck={false}
            value={query}
          />
        </div>
        <PortalledScrollArea
          aria-label={`${ariaLabel} options`}
          className="max-h-60 overflow-y-auto p-1"
          data-testid="channel-combobox-list"
          id={listboxId}
          role="listbox"
        >
          {allowEmpty && !query ? (
            <button
              aria-selected={!value}
              className={cn(
                "flex w-full cursor-pointer items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent hover:text-accent-foreground",
                !value && "bg-accent/50",
              )}
              onClick={() => selectChannel("")}
              role="option"
              type="button"
            >
              <span className="h-5 w-5 shrink-0" />
              <span className="truncate">{emptyLabel}</span>
              <Check
                className={cn(
                  "ml-auto h-4 w-4 shrink-0",
                  !value ? "opacity-100" : "opacity-0",
                )}
              />
            </button>
          ) : null}
          {filtered.length === 0 ? (
            <p className="px-3 py-4 text-center text-xs text-muted-foreground">
              No channels found.
            </p>
          ) : (
            filtered.map((channel) => {
              const optionDisabled = isChannelDisabled?.(channel) ?? false;
              return (
                <button
                  aria-selected={channel.id === value}
                  className={cn(
                    "flex w-full cursor-pointer items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent hover:text-accent-foreground disabled:cursor-not-allowed disabled:opacity-45 disabled:hover:bg-transparent disabled:hover:text-foreground",
                    channel.id === value && "bg-accent/50",
                    channel.id === selectable[highlightedIndex]?.id &&
                      !optionDisabled &&
                      "bg-accent text-accent-foreground",
                  )}
                  disabled={optionDisabled}
                  id={`${listboxId}-option-${channel.id}`}
                  key={channel.id}
                  onClick={() => selectChannel(channel.id)}
                  role="option"
                  type="button"
                >
                  <ChannelPrivacyIcon channel={channel} />
                  <span className="truncate">
                    {channelLabels.get(channel.id) ?? channel.name}
                    {channel.channelType === "dm" ? (
                      <span className="text-muted-foreground"> · dm</span>
                    ) : null}
                  </span>
                  <Check
                    className={cn(
                      "ml-auto h-4 w-4 shrink-0",
                      channel.id === value ? "opacity-100" : "opacity-0",
                    )}
                  />
                </button>
              );
            })
          )}
        </PortalledScrollArea>
      </PopoverContent>
    </Popover>
  );
}
