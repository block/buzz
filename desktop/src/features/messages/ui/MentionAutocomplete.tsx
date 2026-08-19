import * as React from "react";
import { Bot, Pin, Users } from "lucide-react";
import type { TeamMentionMember } from "@/features/messages/lib/mentionCandidates";

import { Badge } from "@/shared/ui/badge";
import { cn } from "@/shared/lib/cn";
import {
  POPOVER_CUSTOM_ENTER_MOTION_CLASS,
  POPOVER_SHADOW_STYLE,
  POPOVER_SURFACE_CLASS,
} from "@/shared/ui/popoverSurface";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { Toggle } from "@/shared/ui/toggle";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { safeNpub } from "@/shared/lib/nostrUtils";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { getPlatformKeysById } from "@/shared/lib/keyboard-shortcuts";

export type MentionSuggestion = {
  pubkey?: string;
  personaId?: string;
  teamId?: string;
  teamMembers?: TeamMentionMember[];
  kind?: "identity" | "persona" | "team";
  displayName: string;
  avatarUrl?: string | null;
  isAgent?: boolean;
  notInChannel?: boolean;
  ownerLabel?: string | null;
  role?: string | null;
};

type MentionAutocompleteProps = {
  suggestions: MentionSuggestion[];
  selectedIndex: number;
  onFetchMore?: () => void;
  onSelect: (suggestion: MentionSuggestion) => void;
  lockedAgentPubkeys?: ReadonlySet<string>;
  onToggleAlwaysAddressAgent?: (suggestion: MentionSuggestion) => void;
  position?: "above" | "below";
};

export const MentionAutocomplete = React.memo(function MentionAutocomplete({
  suggestions,
  selectedIndex,
  onFetchMore,
  onSelect,
  lockedAgentPubkeys,
  onToggleAlwaysAddressAgent,
  position = "above",
}: MentionAutocompleteProps) {
  const listRef = React.useRef<HTMLDivElement>(null);
  const alwaysAddressShortcut = getPlatformKeysById("always-address-agent");

  React.useEffect(() => {
    const activeItem = listRef.current?.querySelector<HTMLElement>(
      `[data-mention-suggestion-index="${selectedIndex}"]`,
    );
    activeItem?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  const handleScroll = React.useCallback(() => {
    const list = listRef.current;
    if (!list || !onFetchMore) return;

    if (list.scrollHeight - list.scrollTop - list.clientHeight < 48) {
      onFetchMore();
    }
  }, [onFetchMore]);

  if (suggestions.length === 0) {
    return null;
  }

  // Name collisions are the impersonation vector: a vanity-ground key can
  // wear any display name. When two suggestions share a name, surface each
  // one's npub (truncated; full key in the hover tooltip) to tell them apart.
  const nameCounts = new Map<string, number>();
  for (const suggestion of suggestions) {
    const name = suggestion.displayName.toLowerCase();
    nameCounts.set(name, (nameCounts.get(name) ?? 0) + 1);
  }

  return (
    <div
      className={cn(
        "absolute left-0 right-0 z-50 px-3 sm:px-4",
        position === "below" ? "top-full mt-1" : "bottom-full mb-1",
      )}
    >
      <div
        className={cn(
          "max-h-48 w-full max-w-2xl overflow-y-auto rounded-xl p-1",
          POPOVER_CUSTOM_ENTER_MOTION_CLASS,
          position === "below"
            ? "origin-top slide-in-from-top-1"
            : "origin-bottom slide-in-from-bottom-1",
          POPOVER_SURFACE_CLASS,
        )}
        data-testid="mention-autocomplete"
        onScroll={handleScroll}
        ref={listRef}
        style={POPOVER_SHADOW_STYLE}
      >
        {suggestions.map((suggestion, index) => {
          const suggestionKey =
            suggestion.pubkey ??
            (suggestion.personaId ? `persona-${suggestion.personaId}` : null) ??
            (suggestion.teamId ? `team-${suggestion.teamId}` : null) ??
            suggestion.displayName;
          const agentLabel = "agent";
          const hasNameCollision =
            (nameCounts.get(suggestion.displayName.toLowerCase()) ?? 0) > 1;
          const collisionNpub =
            hasNameCollision && suggestion.pubkey
              ? safeNpub(suggestion.pubkey)
              : null;
          const canAlwaysAddress = Boolean(
            onToggleAlwaysAddressAgent &&
              suggestion.isAgent &&
              suggestion.pubkey,
          );
          const isAlwaysAddressed = Boolean(
            suggestion.pubkey &&
              lockedAgentPubkeys?.has(suggestion.pubkey.toLowerCase()),
          );

          return (
            <div
              className={cn(
                "flex w-full cursor-pointer items-center gap-2 rounded-lg px-3 py-1.5 text-left text-sm",
                index === selectedIndex
                  ? "bg-accent text-accent-foreground"
                  : "text-popover-foreground hover:bg-accent/50",
              )}
              data-testid={`mention-suggestion-${suggestionKey}`}
              data-mention-suggestion-index={index}
              key={suggestionKey}
            >
              <button
                aria-label={`Mention ${suggestion.displayName}`}
                className="h-6 w-6 shrink-0"
                onMouseDown={(event) => {
                  event.preventDefault();
                  onSelect(suggestion);
                }}
                tabIndex={-1}
                type="button"
              >
                {suggestion.kind === "team" ? (
                  <span className="flex h-6 w-6 items-center justify-center rounded-full bg-primary/10 text-primary">
                    <Users aria-hidden="true" className="h-4 w-4" />
                  </span>
                ) : (
                  <UserAvatar
                    avatarUrl={suggestion.avatarUrl ?? null}
                    displayName={suggestion.displayName}
                    size="xs"
                    testId="mention-suggestion-avatar"
                  />
                )}
              </button>
              <button
                className="flex min-w-0 flex-1 items-center gap-2 text-left"
                onMouseDown={(event) => {
                  event.preventDefault();
                  onSelect(suggestion);
                }}
                tabIndex={-1}
                type="button"
              >
                <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <span
                    className="min-w-0 break-words font-medium leading-snug"
                    title={suggestion.displayName}
                  >
                    {suggestion.displayName}
                  </span>
                  {suggestion.kind === "team" ||
                  suggestion.isAgent ||
                  suggestion.role ||
                  suggestion.ownerLabel ||
                  suggestion.notInChannel ? (
                    <span
                      className={cn(
                        "flex min-w-0 items-center gap-1.5 text-2xs leading-none",
                        index === selectedIndex
                          ? "text-accent-foreground/60"
                          : "text-muted-foreground",
                      )}
                    >
                      {suggestion.kind === "team" ? (
                        <span className="inline-flex shrink-0 items-center gap-1">
                          <Users aria-hidden="true" className="h-3.5 w-3.5" />
                          team · {suggestion.teamMembers?.length ?? 0} agents
                        </span>
                      ) : suggestion.isAgent ? (
                        <span className="inline-flex shrink-0 items-center gap-1">
                          <Bot
                            aria-hidden="true"
                            className="h-3.5 w-3.5"
                            data-testid="mention-agent-icon"
                          />
                          {agentLabel}
                        </span>
                      ) : suggestion.role ? (
                        <Badge
                          className="max-w-24 shrink-0 truncate"
                          variant="secondary"
                        >
                          {suggestion.role}
                        </Badge>
                      ) : null}
                      {suggestion.ownerLabel || suggestion.notInChannel ? (
                        <span
                          className="min-w-0 truncate"
                          title={
                            suggestion.ownerLabel && suggestion.notInChannel
                              ? `managed by ${suggestion.ownerLabel} · not in channel`
                              : suggestion.ownerLabel
                                ? `managed by ${suggestion.ownerLabel}`
                                : "not in channel"
                          }
                        >
                          {suggestion.ownerLabel && suggestion.notInChannel
                            ? `managed by ${suggestion.ownerLabel} · not in channel`
                            : suggestion.ownerLabel
                              ? `managed by ${suggestion.ownerLabel}`
                              : "not in channel"}
                        </span>
                      ) : null}
                    </span>
                  ) : null}
                  {collisionNpub ? (
                    <span
                      className={cn(
                        "min-w-0 truncate font-mono text-2xs leading-snug",
                        index === selectedIndex
                          ? "text-accent-foreground/60"
                          : "text-muted-foreground",
                      )}
                      data-testid="mention-collision-npub"
                      title={collisionNpub}
                    >
                      {truncatePubkey(collisionNpub)}
                    </span>
                  ) : null}
                </span>
              </button>
              {canAlwaysAddress ? (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <span className="inline-flex shrink-0">
                      <Toggle
                        aria-label={`Always address ${suggestion.displayName}`}
                        className="h-6 w-6 p-0 data-[state=on]:bg-primary/15 data-[state=on]:text-primary"
                        data-always-address-pubkey={suggestion.pubkey?.toLowerCase()}
                        data-testid={`mention-always-address-${suggestion.pubkey}`}
                        onPressedChange={() =>
                          onToggleAlwaysAddressAgent?.(suggestion)
                        }
                        onClick={(event) => event.stopPropagation()}
                        onMouseDown={(event) => {
                          event.preventDefault();
                          event.stopPropagation();
                        }}
                        pressed={isAlwaysAddressed}
                        size="xs"
                        type="button"
                      >
                        <Pin aria-hidden="true" />
                      </Toggle>
                    </span>
                  </TooltipTrigger>
                  <TooltipContent
                    className="flex flex-col items-center gap-1.5"
                    side="top"
                  >
                    <span>Always address</span>
                    {alwaysAddressShortcut ? (
                      <span className="flex items-center gap-1">
                        {alwaysAddressShortcut.split("+").map((key, index) => (
                          <React.Fragment key={key}>
                            {index > 0 ? (
                              <span className="text-primary-foreground/50">
                                +
                              </span>
                            ) : null}
                            <kbd className="rounded border border-primary-foreground/20 bg-primary-foreground/10 px-1 py-0.5 font-mono text-2xs text-primary-foreground/70">
                              {key}
                            </kbd>
                          </React.Fragment>
                        ))}
                      </span>
                    ) : null}
                  </TooltipContent>
                </Tooltip>
              ) : null}
            </div>
          );
        })}
      </div>
    </div>
  );
});
