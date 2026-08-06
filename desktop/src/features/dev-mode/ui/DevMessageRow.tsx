import * as React from "react";

import { useCustomEmoji } from "@/features/custom-emoji/hooks";
import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import type { AuthorColorResolver } from "@/features/dev-mode/lib/authorColors";
import { useChannelRefs } from "@/features/dev-mode/lib/channelRefs";
import { renderDevMarkdown } from "@/features/dev-mode/lib/devMarkdown";
import {
  matchLeadingMention,
  type MentionStyle,
} from "@/features/dev-mode/lib/highlightContent";
import {
  type AgentStatusSummary,
  applyReactionToggle,
  groupReactions,
  type MessageReaction,
  type ReactionGroup,
  splitAgentStatus,
} from "@/features/dev-mode/lib/messageReactions";
import { DevSpotlightText } from "@/features/dev-mode/ui/DevSpotlightText";
import type {
  AgentResolver,
  NameResolver,
} from "@/features/dev-mode/lib/useMemberNameResolver";
import { useToggleReactionMutation } from "@/features/messages/hooks";
import { reactionEmojiUrl } from "@/shared/api/customEmoji";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { emojiDisplayName } from "@/shared/lib/emojiName";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { useMediaProxyPort } from "@/shared/lib/useMediaProxyPort";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { parseImetaTags } from "@/shared/ui/markdown/parseImeta";

function formatTime(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}

function reactionTitle(
  group: ReactionGroup,
  resolveName: NameResolver,
  mine: boolean,
): string {
  const names = [...new Set(group.pubkeys.map(resolveName))].join(", ");
  const base = `${emojiDisplayName(group.emoji)} — ${names}`;
  return mine ? `${base} (click to remove)` : base;
}

function ReactionGlyph({ group }: { group: ReactionGroup }) {
  if (!group.emojiUrl) {
    return <>{group.emoji}</>;
  }
  // Relay media must go through the localhost proxy (VPN bypass) like every
  // other relay-hosted <img>.
  return (
    <img
      alt={group.emoji}
      src={rewriteRelayUrl(group.emojiUrl)}
      className="inline-block h-4 w-4 -translate-y-px object-contain align-text-bottom"
      draggable={false}
    />
  );
}

/**
 * Per-message agent activity, derived from the harness's 👀/💬 status
 * reactions but rendered as text instead of emoji chips: a dim "queued"
 * while the prompt waits and a spotlight-swept "working…" (the same sweep
 * working channel names get) while an agent is on it.
 */
function AgentStatusIndicator({
  summary,
  resolveName,
}: {
  summary: AgentStatusSummary;
  resolveName: NameResolver;
}) {
  const names = [...new Set(summary.pubkeys.map(resolveName))].join(", ");
  if (summary.status === "working") {
    return (
      <span
        className="shrink-0 select-none text-xs text-muted-foreground"
        data-testid="dev-mode-agent-status-working"
      >
        <DevSpotlightText text={`${names} working…`} />
      </span>
    );
  }
  return (
    <span
      className="shrink-0 select-none text-xs text-muted-foreground/50"
      data-testid="dev-mode-agent-status-queued"
    >
      {names} queued
    </span>
  );
}

function DevReactions({
  eventId,
  canReact,
  reactions,
  currentPubkey,
  resolveName,
}: {
  eventId: string;
  canReact: boolean;
  reactions: MessageReaction[] | undefined;
  currentPubkey: string | null;
  resolveName: NameResolver;
}) {
  const customEmoji = useCustomEmoji();
  const toggleReaction = useToggleReactionMutation();
  const [pickerOpen, setPickerOpen] = React.useState(false);
  // Re-render when the media proxy port resolves so a first-paint
  // buzz-media:// fallback src upgrades to the loopback proxy URL.
  useMediaProxyPort();

  // Reactions published without the NIP-30 tag (e.g. via the CLI) still
  // resolve their image through the community palette.
  const sourceGroups = React.useMemo(
    () =>
      groupReactions(reactions).map((group) =>
        group.emojiUrl
          ? group
          : { ...group, emojiUrl: reactionEmojiUrl(group.emoji, customEmoji) },
      ),
    [reactions, customEmoji],
  );

  // Optimistic overlay: valid only while the source reactions haven't
  // changed under us; any relay update (including our own echo) wins.
  const [optimistic, setOptimistic] = React.useState<{
    source: MessageReaction[] | undefined;
    groups: ReactionGroup[];
  } | null>(null);
  const groups =
    optimistic && optimistic.source === reactions
      ? optimistic.groups
      : sourceGroups;

  const toggle = (emoji: string) => {
    if (!canReact || !currentPubkey) return;
    const remove =
      groups
        .find((group) => group.emoji === emoji)
        ?.pubkeys.includes(currentPubkey) ?? false;
    setOptimistic({
      source: reactions,
      groups: applyReactionToggle(
        groups,
        emoji,
        currentPubkey,
        reactionEmojiUrl(emoji, customEmoji),
      ),
    });
    toggleReaction.mutate(
      { eventId, emoji, remove },
      { onError: () => setOptimistic(null) },
    );
  };

  if (groups.length === 0 && !canReact) {
    return null;
  }

  return (
    <span className="flex min-w-0 select-none flex-wrap items-baseline gap-2">
      {groups.map((group) => {
        const mine =
          currentPubkey !== null && group.pubkeys.includes(currentPubkey);
        return (
          <button
            key={group.emoji}
            aria-label={`Toggle ${group.emoji} reaction`}
            aria-pressed={mine}
            className={cn(
              "text-xs",
              mine ? "text-primary" : "text-muted-foreground",
              canReact && "cursor-pointer hover:text-foreground",
            )}
            onClick={() => toggle(group.emoji)}
            title={reactionTitle(group, resolveName, mine)}
            type="button"
          >
            <ReactionGlyph group={group} />
            {group.pubkeys.length > 1 ? ` ${group.pubkeys.length}` : ""}
          </button>
        );
      })}
      {canReact ? (
        <Popover onOpenChange={setPickerOpen} open={pickerOpen}>
          <PopoverTrigger asChild>
            <button
              aria-label="Add reaction"
              className={cn(
                "cursor-pointer px-1 text-xs text-muted-foreground/70 hover:text-foreground",
                pickerOpen
                  ? "opacity-100"
                  : "opacity-0 focus-visible:opacity-100 group-hover/devrow:opacity-100",
              )}
              data-testid={`dev-mode-add-reaction-${eventId}`}
              type="button"
            >
              +
            </button>
          </PopoverTrigger>
          <PopoverContent
            align="start"
            className="w-auto overflow-hidden rounded-2xl border-0 bg-transparent p-0 shadow-none"
            side="bottom"
            sideOffset={4}
          >
            <EmojiPicker
              autoFocus
              onSelect={(emoji) => {
                toggle(emoji);
                setPickerOpen(false);
              }}
            />
          </PopoverContent>
        </Popover>
      ) : null}
    </span>
  );
}

export function DevMessageRow({
  event,
  currentPubkey,
  edited = false,
  reactions,
  resolveName,
  resolveColor,
  resolveIsAgent,
}: {
  event: RelayEvent;
  currentPubkey: string | null;
  /** Whether a kind:40003 edit has been applied to this message's content. */
  edited?: boolean;
  /**
   * Emoji reacted onto this message. Agent-published 👀/💬 harness status
   * reactions are split out and rendered as a text activity indicator; the
   * rest render as ordinary chips.
   */
  reactions?: MessageReaction[];
  resolveName: NameResolver;
  resolveColor: AuthorColorResolver;
  resolveIsAgent: AgentResolver;
}) {
  const isSelf = event.pubkey === currentPubkey;
  const { channels, openChannel } = useChannelRefs();
  // Stable per-event identity so the media renderer's memo holds.
  const imetaByUrl = React.useMemo(
    () => parseImetaTags(event.tags),
    [event.tags],
  );
  const { status: agentStatus, rest: chipReactions } = React.useMemo(
    () => splitAgentStatus(reactions, resolveIsAgent),
    [reactions, resolveIsAgent],
  );

  if (event.kind === KIND_SYSTEM_MESSAGE) {
    return null;
  }

  // The pubkeys this message explicitly mentions (its p tags) let known
  // `@Name` tokens render as pills in the mentioned author's color.
  const mentionStyles: MentionStyle[] = [];
  for (const tag of event.tags) {
    if (tag[0] !== "p" || !tag[1]) continue;
    const name = resolveName(tag[1]);
    if (mentionStyles.some((mention) => mention.name === name)) continue;
    mentionStyles.push({ name, color: resolveColor(tag[1]) });
  }

  // A leading `@Name` mention on a human message is direction, not prose:
  // it renders as a "to Name" line under the author instead of inside the
  // message body. Agent replies keep their mentions inline as normal text.
  const directed = resolveIsAgent(event.pubkey)
    ? null
    : matchLeadingMention(event.content, mentionStyles);
  const bodyContent = directed
    ? event.content.slice(directed.end)
    : event.content;

  return (
    <div className="group/devrow min-w-0 py-1 text-sm leading-6">
      <div className="flex min-w-0 items-baseline gap-2">
        <span
          className={cn(
            "shrink-0 font-medium",
            isSelf && "underline decoration-dotted underline-offset-4",
          )}
          style={{ color: resolveColor(event.pubkey) }}
        >
          {resolveName(event.pubkey)}
        </span>
        {directed ? (
          <span className="shrink-0 select-none text-xs text-muted-foreground/60">
            to{" "}
            <span style={{ color: directed.mention.color }}>
              {directed.mention.name}
            </span>
          </span>
        ) : null}
        <span className="shrink-0 select-none text-xs text-muted-foreground/50">
          {formatTime(event.created_at)}
        </span>
        {edited ? (
          <span className="shrink-0 select-none text-xs text-muted-foreground/40">
            (edited)
          </span>
        ) : null}
        {agentStatus ? (
          <AgentStatusIndicator
            resolveName={resolveName}
            summary={agentStatus}
          />
        ) : null}
        <DevReactions
          canReact={currentPubkey !== null && !event.pending}
          currentPubkey={currentPubkey}
          eventId={event.id}
          reactions={chipReactions}
          resolveName={resolveName}
        />
      </div>
      <div
        className={cn(
          "min-w-0 space-y-1 break-words [overflow-wrap:anywhere]",
          event.pending && "text-muted-foreground",
        )}
      >
        {renderDevMarkdown(
          bodyContent,
          mentionStyles,
          { channels, onOpen: openChannel },
          imetaByUrl,
        )}
      </div>
    </div>
  );
}
