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
  currentScopeActionLabel?: string;
  inputRef: React.RefObject<HTMLInputElement | null>;
  onActivateCurrentScope?: () => void;
  onChange: (query: string) => void;
  onKeyDown: React.KeyboardEventHandler<HTMLInputElement>;
  onRemoveScope: () => void;
  query: string;
  scopeLabel: string | null;
};

export function SearchDialogInputRow({
  currentScopeActionLabel,
  inputRef,
  onActivateCurrentScope,
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
      {currentScopeActionLabel && onActivateCurrentScope && !scopeLabel ? (
        <button
          aria-label={`${currentScopeActionLabel} (Tab)`}
          className="flex shrink-0 items-center gap-2 rounded-md px-1.5 py-1 text-xs font-medium text-muted-foreground transition-colors hover:bg-muted/70 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
          data-testid="search-current-channel-control"
          onClick={onActivateCurrentScope}
          type="button"
        >
          <span>{currentScopeActionLabel}</span>
          <kbd className="rounded border border-border/70 bg-muted/70 px-1.5 py-0.5 text-2xs text-muted-foreground">
            TAB
          </kbd>
        </button>
      ) : null}
    </div>
  );
}
