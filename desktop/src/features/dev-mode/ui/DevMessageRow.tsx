import * as React from "react";

import type { AuthorColorResolver } from "@/features/dev-mode/lib/authorColors";
import { useChannelRefs } from "@/features/dev-mode/lib/channelRefs";
import { renderDevMarkdown } from "@/features/dev-mode/lib/devMarkdown";
import {
  matchLeadingMention,
  type MentionStyle,
} from "@/features/dev-mode/lib/highlightContent";
import type { MessageReaction } from "@/features/dev-mode/lib/messageReactions";
import type {
  AgentResolver,
  NameResolver,
} from "@/features/dev-mode/lib/useMemberNameResolver";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { parseImetaTags } from "@/shared/ui/markdown/parseImeta";

function formatTime(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}

function ReactionChips({
  reactions,
  resolveName,
}: {
  reactions: MessageReaction[];
  resolveName: NameResolver;
}) {
  const byEmoji = new Map<string, string[]>();
  for (const { emoji, pubkey } of reactions) {
    const bucket = byEmoji.get(emoji);
    if (bucket) {
      bucket.push(pubkey);
    } else {
      byEmoji.set(emoji, [pubkey]);
    }
  }
  return (
    <span className="flex shrink-0 select-none items-baseline gap-1 self-start">
      {[...byEmoji.entries()].map(([emoji, pubkeys]) => (
        <span
          key={emoji}
          className="text-xs text-muted-foreground"
          title={[...new Set(pubkeys.map(resolveName))].join(", ")}
        >
          {emoji}
          {pubkeys.length > 1 ? ` ${pubkeys.length}` : ""}
        </span>
      ))}
    </span>
  );
}

export function DevMessageRow({
  event,
  isSelf,
  reactions,
  resolveName,
  resolveColor,
  resolveIsAgent,
}: {
  event: RelayEvent;
  isSelf: boolean;
  /** Emoji reacted onto this message — agents react while working, so this doubles as the loading state. */
  reactions?: MessageReaction[];
  resolveName: NameResolver;
  resolveColor: AuthorColorResolver;
  resolveIsAgent: AgentResolver;
}) {
  const { channels, openChannel } = useChannelRefs();
  // Stable per-event identity so the media renderer's memo holds.
  const imetaByUrl = React.useMemo(
    () => parseImetaTags(event.tags),
    [event.tags],
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
    <div className="min-w-0 py-1 text-sm leading-6">
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
        {reactions && reactions.length > 0 ? (
          <ReactionChips reactions={reactions} resolveName={resolveName} />
        ) : null}
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
