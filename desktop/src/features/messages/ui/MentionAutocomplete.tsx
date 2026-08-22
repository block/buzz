import * as React from "react";
import { AtSign, Bot, ChevronRight, Users } from "lucide-react";
import { OtherSetupAgentMarker } from "@/features/agents/ui/OtherSetupAgentMarker";
import { motion } from "motion/react";
import type { TeamMentionMember } from "@/features/messages/lib/mentionCandidates";

import { Badge } from "@/shared/ui/badge";
import { cn } from "@/shared/lib/cn";
import {
  POPOVER_CUSTOM_ENTER_MOTION_CLASS,
  POPOVER_SHADOW_STYLE,
  POPOVER_SURFACE_CLASS,
} from "@/shared/ui/popoverSurface";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { Switch } from "@/shared/ui/switch";
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
  agentProvenance?: "managed-here" | "managed-elsewhere";
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
  keepMentionedAgentsPinned?: boolean;
  onKeepMentionedAgentsPinnedChange?: (value: boolean) => void;
  openOptionsRequest?: number;
  onDismiss?: () => void;
  position?: "above" | "below";
};

export function showMentionAgentProvenanceMarker(
  suggestion: MentionSuggestion,
  hasNameCollision: boolean,
): boolean {
  return hasNameCollision && suggestion.agentProvenance === "managed-elsewhere";
}

export const MentionAutocomplete = React.memo(function MentionAutocomplete({
  suggestions,
  selectedIndex,
  onFetchMore,
  onSelect,
  lockedAgentPubkeys,
  onToggleAlwaysAddressAgent,
  keepMentionedAgentsPinned = true,
  onKeepMentionedAgentsPinnedChange,
  openOptionsRequest = 0,
  onDismiss,
  position = "above",
}: MentionAutocompleteProps) {
  const rootRef = React.useRef<HTMLDivElement>(null);
  const optionsSurfaceRef = React.useRef<HTMLDivElement>(null);
  const listRef = React.useRef<HTMLDivElement>(null);
  const optionsId = React.useId();
  const keepPinnedSwitchId = React.useId();
  const [optionsOpen, setOptionsOpen] = React.useState(false);
  const alwaysAddressShortcut = getPlatformKeysById("always-address-agent");

  React.useEffect(() => {
    const activeItem = listRef.current?.querySelector<HTMLElement>(
      `[data-mention-suggestion-index="${selectedIndex}"]`,
    );
    activeItem?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  React.useEffect(() => {
    if (suggestions.length === 0) {
      setOptionsOpen(false);
    }
  }, [suggestions.length]);

  React.useEffect(() => {
    if (openOptionsRequest > 0) {
      setOptionsOpen(true);
    }
  }, [openOptionsRequest]);

  React.useEffect(() => {
    if (!onDismiss) return;

    const handlePointerDown = (event: PointerEvent) => {
      const root = rootRef.current;
      const target = event.target;
      if (!root || !(target instanceof Node)) return;
      if (
        listRef.current?.contains(target) ||
        optionsSurfaceRef.current?.contains(target)
      ) {
        return;
      }

      const composer = root.closest("form");
      const mentionTrigger =
        target instanceof Element
          ? target.closest("[data-mention-picker-trigger]")
          : null;
      if (composer && mentionTrigger && composer.contains(mentionTrigger)) {
        return;
      }

      onDismiss();
    };

    document.addEventListener("pointerdown", handlePointerDown, true);
    return () =>
      document.removeEventListener("pointerdown", handlePointerDown, true);
  }, [onDismiss]);

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
      data-testid="mention-autocomplete-layer"
      ref={rootRef}
    >
      <div className="w-full max-w-2xl">
        {onKeepMentionedAgentsPinnedChange ? (
          <div className="mb-2 flex justify-end">
            <div
              className={cn(
                "max-w-full overflow-hidden rounded-xl text-popover-foreground ring-1 ring-border/50 transition-[width] duration-200 ease-out",
                POPOVER_SURFACE_CLASS,
                optionsOpen ? "w-72" : "w-24",
              )}
              ref={optionsSurfaceRef}
              style={POPOVER_SHADOW_STYLE}
            >
              {optionsOpen ? (
                <motion.div
                  animate={{ opacity: 1, y: 0 }}
                  className="w-72"
                  id={optionsId}
                  initial={{ opacity: 0, y: 4 }}
                  transition={{ duration: 0.16, ease: "easeOut" }}
                >
                  <div className="flex min-h-12 items-center justify-between gap-3 px-3 py-2">
                    <label
                      className="flex min-w-0 flex-col"
                      htmlFor={keepPinnedSwitchId}
                    >
                      <span className="text-sm font-medium">
                        Automatically mention agents
                      </span>
                      <span className="text-2xs text-muted-foreground">
                        After you mention them once
                      </span>
                    </label>
                    <Switch
                      aria-label="Automatically mention agents"
                      checked={keepMentionedAgentsPinned}
                      className="shadow-none [&>span]:shadow-none"
                      data-testid="mention-keep-agents-pinned-toggle"
                      id={keepPinnedSwitchId}
                      onCheckedChange={onKeepMentionedAgentsPinnedChange}
                      onMouseDown={(event) => event.preventDefault()}
                    />
                  </div>
                  <div className="mx-3 border-t border-border/50" />
                </motion.div>
              ) : null}
              <button
                aria-controls={optionsId}
                aria-expanded={optionsOpen}
                className="flex h-8 w-full items-center justify-between gap-1 px-2.5 text-sm font-medium text-muted-foreground transition-colors hover:bg-accent hover:text-popover-foreground"
                data-testid="mention-options-trigger"
                onClick={() => setOptionsOpen((open) => !open)}
                onMouseDown={(event) => event.preventDefault()}
                type="button"
              >
                <span>Options</span>
                <motion.span
                  animate={{ rotate: optionsOpen ? -90 : 0 }}
                  className="inline-flex"
                  transition={{ duration: 0.16, ease: "easeOut" }}
                >
                  <ChevronRight aria-hidden="true" className="h-3.5 w-3.5" />
                </motion.span>
              </button>
            </div>
          </div>
        ) : null}
        <div
          className={cn(
            "max-h-48 w-full overflow-y-auto rounded-xl p-1",
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
              (suggestion.personaId
                ? `persona-${suggestion.personaId}`
                : null) ??
              (suggestion.teamId ? `team-${suggestion.teamId}` : null) ??
              suggestion.displayName;
            const hasNameCollision =
              (nameCounts.get(suggestion.displayName.toLowerCase()) ?? 0) > 1;
            const showAgentProvenanceMarker = showMentionAgentProvenanceMarker(
              suggestion,
              hasNameCollision,
            );
            const ownerLabel =
              hasNameCollision && suggestion.agentProvenance
                ? null
                : suggestion.ownerLabel;
            const collisionNpub =
              hasNameCollision && suggestion.pubkey
                ? safeNpub(suggestion.pubkey)
                : null;
            const hasMetadataBeforeNpub = Boolean(
              suggestion.kind === "team" ||
                suggestion.isAgent ||
                suggestion.role ||
                ownerLabel ||
                suggestion.notInChannel,
            );
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
                  "relative flex w-full items-stretch rounded-lg text-left text-sm",
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
                  className={cn(
                    "flex min-w-0 flex-1 items-center gap-2 rounded-lg px-3 py-1.5 text-left",
                    canAlwaysAddress && "pr-11",
                  )}
                  onMouseDown={(event) => {
                    event.preventDefault();
                    onSelect(suggestion);
                  }}
                  tabIndex={-1}
                  type="button"
                >
                  {suggestion.kind === "team" ? (
                    <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-primary/10 text-primary">
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
                  <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <span
                      className="min-w-0 break-words font-medium leading-snug"
                      title={suggestion.displayName}
                    >
                      {suggestion.displayName}
                    </span>
                    {hasMetadataBeforeNpub || collisionNpub ? (
                      <span
                        className={cn(
                          "flex min-h-3.5 min-w-0 items-center gap-1.5 text-2xs leading-none",
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
                            agent
                            {showAgentProvenanceMarker ? (
                              <OtherSetupAgentMarker testId="mention-agent-provenance" />
                            ) : null}
                          </span>
                        ) : suggestion.role ? (
                          <Badge
                            className="max-w-24 shrink-0 truncate"
                            variant="secondary"
                          >
                            {suggestion.role}
                          </Badge>
                        ) : null}
                        {ownerLabel || suggestion.notInChannel ? (
                          <span
                            className="min-w-0 truncate"
                            title={
                              ownerLabel && suggestion.notInChannel
                                ? `managed by ${ownerLabel} · not in channel`
                                : ownerLabel
                                  ? `managed by ${ownerLabel}`
                                  : "not in channel"
                            }
                          >
                            {ownerLabel && suggestion.notInChannel
                              ? `managed by ${ownerLabel} · not in channel`
                              : ownerLabel
                                ? `managed by ${ownerLabel}`
                                : "not in channel"}
                          </span>
                        ) : null}
                        {collisionNpub ? (
                          <span
                            className="-translate-y-0.5 shrink-0 font-mono leading-none"
                            data-testid="mention-collision-npub"
                            title={collisionNpub}
                          >
                            {truncatePubkey(collisionNpub)}
                          </span>
                        ) : null}
                      </span>
                    ) : null}
                  </span>
                </button>
                {canAlwaysAddress ? (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span className="absolute right-3 top-1/2 inline-flex -translate-y-1/2">
                        <Toggle
                          aria-label={`${isAlwaysAddressed ? "Stop automatically mentioning" : "Automatically mention"} ${suggestion.displayName}`}
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
                          <AtSign aria-hidden="true" className="h-3.5 w-3.5" />
                        </Toggle>
                      </span>
                    </TooltipTrigger>
                    <TooltipContent
                      className="flex items-center gap-2"
                      side="top"
                    >
                      <span>
                        {isAlwaysAddressed
                          ? "Stop automatically mentioning"
                          : "Automatically mention"}
                      </span>
                      {alwaysAddressShortcut ? (
                        <kbd className="flex items-center gap-0.5 rounded border border-primary-foreground/20 bg-primary-foreground/10 px-1 py-0 font-mono text-sm text-primary-foreground/70">
                          {(alwaysAddressShortcut.includes("+")
                            ? alwaysAddressShortcut.split("+")
                            : Array.from(alwaysAddressShortcut)
                          ).map((key) => (
                            <span key={key}>{key}</span>
                          ))}
                        </kbd>
                      ) : null}
                    </TooltipContent>
                  </Tooltip>
                ) : null}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
});
