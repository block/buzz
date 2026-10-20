import { Search, X } from "lucide-react";
import type * as React from "react";

import { buildDirectMessageIntro } from "@/features/channels/lib/dmParticipantDisplay";
import { SearchPromptPlaceholder } from "@/features/search/ui/SearchPromptPlaceholder";
import type { Channel } from "@/shared/api/types";

export function getChannelScopeLabel(
  channel: Channel,
  channelLabels?: Record<string, string>,
  currentPubkey?: string,
) {
  const name = channelLabels?.[channel.id]?.trim() || channel.name;
  if (channel.channelType !== "dm") {
    return `#${name}`;
  }

  const participantLabel = buildDirectMessageIntro({
    channel,
    currentPubkey,
  })?.displayName;
  const hasResolvedChannelLabel =
    channelLabels?.[channel.id]?.trim() && name !== channel.name.trim();

  return hasResolvedChannelLabel ? name : participantLabel || name;
}

type SearchDialogInputRowProps = {
  inputRef: React.RefObject<HTMLInputElement | null>;
  onChange: (query: string) => void;
  onKeyDown: React.KeyboardEventHandler<HTMLInputElement>;
  onRemoveScope: () => void;
  query: string;
  scopeLabel: string | null;
};

export function SearchDialogInputRow({
  inputRef,
  onChange,
  onKeyDown,
  onRemoveScope,
  query,
  scopeLabel,
}: SearchDialogInputRowProps) {
  return (
    <div
      className="flex h-12 items-center gap-3 border-b border-border/70 px-4"
      data-testid="search-dialog-input-row"
    >
      <Search className="h-4 w-4 shrink-0 text-muted-foreground" />
      {scopeLabel ? (
        <button
          aria-label={`Remove ${scopeLabel} search scope`}
          className="flex h-7 max-w-48 shrink-0 items-center gap-1 rounded-md border border-primary/20 bg-primary/10 px-2 text-sm font-medium text-foreground transition-colors hover:bg-primary/15 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
          data-testid="search-channel-scope-chip"
          onClick={onRemoveScope}
          title={`Remove ${scopeLabel} search scope`}
          type="button"
        >
          <span className="truncate">{scopeLabel}</span>
          <X className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
        </button>
      ) : null}
      <div className="relative min-w-0 flex-1">
        {query.length === 0 ? (
          <span className="pointer-events-none absolute inset-y-0 left-0 flex items-center text-base leading-none">
            {scopeLabel ? (
              <span className="text-muted-foreground">Search messages</span>
            ) : (
              <SearchPromptPlaceholder />
            )}
          </span>
        ) : null}
        <input
          aria-label={
            scopeLabel ? `Search in ${scopeLabel}` : "Search everything"
          }
          autoCapitalize="none"
          autoCorrect="off"
          className="relative z-10 w-full min-w-0 bg-transparent text-base text-foreground outline-none"
          data-testid="search-dialog-input"
          ref={inputRef}
          onChange={(event) => onChange(event.target.value)}
          onKeyDown={onKeyDown}
          spellCheck={false}
          value={query}
        />
      </div>
      <kbd className="shrink-0 rounded border border-border/70 bg-muted/70 px-1.5 py-0.5 text-2xs text-muted-foreground">
        ESC
      </kbd>
    </div>
  );
}

type CurrentChannelSearchActionProps = {
  channelLabel: string;
  channelType: Channel["channelType"];
  isSelected: boolean;
  onActivate: () => void;
  onMouseEnter: () => void;
};

export function CurrentChannelSearchAction({
  channelLabel,
  channelType,
  isSelected,
  onActivate,
  onMouseEnter,
}: CurrentChannelSearchActionProps) {
  const isDirectMessage = channelType === "dm";

  return (
    <div className="px-3 py-3.5">
      <button
        aria-selected={isSelected}
        className={`flex w-full items-center gap-4 rounded-lg border border-border/75 px-3 py-3 text-left transition-colors focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring ${
          isSelected ? "bg-muted/55" : "bg-muted/30 hover:bg-muted/55"
        }`}
        data-search-result-index="0"
        data-testid="search-current-channel-control"
        onClick={onActivate}
        onMouseEnter={onMouseEnter}
        role="option"
        type="button"
      >
        <span className="flex min-w-0 items-center gap-2">
          <Search className="h-3.5 w-3.5 shrink-0" />
          <span
            className="min-w-0 truncate text-sm text-muted-foreground"
            data-testid="search-current-scope-label"
          >
            {isDirectMessage ? "Search conversation with " : "Search in "}
            <span className="font-medium text-foreground">{channelLabel}</span>
          </span>
        </span>
        <span className="ml-auto shrink-0 text-xs text-muted-foreground/70">
          {isDirectMessage
            ? "Search messages in this conversation."
            : "Search messages in this channel"}
        </span>
      </button>
    </div>
  );
}
