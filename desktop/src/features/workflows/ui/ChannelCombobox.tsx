import { Check, ChevronsUpDown, Search } from "lucide-react";
import * as React from "react";

import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";

function formatChannelLabel(ch: Channel): string {
  return `${ch.name} · ${ch.channelType} · ${ch.visibility}`;
}

type ChannelComboboxProps = {
  channels: Channel[];
  disabled?: boolean;
  id?: string;
  onChange: (value: string) => void;
  value: string;
};

export function ChannelCombobox({
  channels,
  disabled,
  id,
  onChange,
  value,
}: ChannelComboboxProps) {
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const [highlightedIndex, setHighlightedIndex] = React.useState(0);

  const selected = channels.find((c) => c.id === value);

  const filtered = React.useMemo(() => {
    if (!query) return channels;
    const q = query.toLowerCase();
    return channels.filter(
      (c) =>
        c.name.toLowerCase().includes(q) ||
        c.channelType?.toLowerCase().includes(q) ||
        c.id.toLowerCase().includes(q),
    );
  }, [channels, query]);

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
    if (filtered.length === 0) return;

    switch (e.key) {
      case "ArrowDown": {
        e.preventDefault();
        setHighlightedIndex((i) => (i + 1) % filtered.length);
        break;
      }
      case "ArrowUp": {
        e.preventDefault();
        setHighlightedIndex((i) => (i - 1 + filtered.length) % filtered.length);
        break;
      }
      case "Enter": {
        e.preventDefault();
        const target = filtered[highlightedIndex];
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

  return (
    <Popover onOpenChange={handleOpenChange} open={open}>
      <PopoverTrigger asChild>
        <button
          aria-expanded={open}
          className={cn(
            "flex h-9 w-full items-center justify-between rounded-md border border-input bg-transparent px-3 text-sm shadow-xs transition-colors focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
            !selected && "text-muted-foreground",
          )}
          disabled={disabled}
          id={id}
          role="combobox"
          type="button"
        >
          <span className="truncate">
            {selected ? formatChannelLabel(selected) : "Select a channel..."}
          </span>
          <ChevronsUpDown className="ml-2 h-4 w-4 shrink-0 text-muted-foreground" />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-(--radix-popover-trigger-width) p-0"
      >
        <div className="flex items-center gap-2 border-b border-border px-3 py-2">
          <Search className="h-4 w-4 shrink-0 text-muted-foreground" />
          <input
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
            spellCheck={false}
            value={query}
          />
        </div>
        <div className="max-h-60 overflow-y-auto p-1">
          {filtered.length === 0 ? (
            <p className="px-3 py-4 text-center text-xs text-muted-foreground">
              No channels found.
            </p>
          ) : (
            filtered.map((channel, index) => (
              <button
                className={cn(
                  "flex w-full items-center gap-2 rounded-xs px-2 py-1.5 text-left text-sm transition-colors hover:bg-accent hover:text-accent-foreground",
                  channel.id === value && "bg-accent/50",
                  index === highlightedIndex &&
                    "bg-accent text-accent-foreground",
                )}
                key={channel.id}
                onClick={() => selectChannel(channel.id)}
                type="button"
              >
                <Check
                  className={cn(
                    "h-4 w-4 shrink-0",
                    channel.id === value ? "opacity-100" : "opacity-0",
                  )}
                />
                <span className="truncate">
                  {channel.name}{" "}
                  <span className="text-muted-foreground">
                    · {channel.channelType} · {channel.visibility}
                  </span>
                </span>
              </button>
            ))
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}
