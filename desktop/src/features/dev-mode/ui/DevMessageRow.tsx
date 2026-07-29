import type { AuthorColorResolver } from "@/features/dev-mode/lib/authorColors";
import { useChannelRefs } from "@/features/dev-mode/lib/channelRefs";
import {
  matchLeadingMention,
  renderHighlightedContent,
  type MentionStyle,
} from "@/features/dev-mode/lib/highlightContent";
import type { NameResolver } from "@/features/dev-mode/lib/useMemberNameResolver";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";

function formatTime(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}

function ReactionChips({ reactions }: { reactions: string[] }) {
  const counts = new Map<string, number>();
  for (const emoji of reactions) {
    counts.set(emoji, (counts.get(emoji) ?? 0) + 1);
  }
  return (
    <span className="flex shrink-0 select-none items-baseline gap-1 self-start">
      {[...counts.entries()].map(([emoji, count]) => (
        <span
          key={emoji}
          className="rounded-none border border-border/50 bg-muted/40 px-1 text-xs text-muted-foreground"
        >
          {emoji}
          {count > 1 ? ` ${count}` : ""}
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
}: {
  event: RelayEvent;
  isSelf: boolean;
  /** Emoji reacted onto this message — agents react while working, so this doubles as the loading state. */
  reactions?: string[];
  resolveName: NameResolver;
  resolveColor: AuthorColorResolver;
}) {
  const { channels, openChannel } = useChannelRefs();

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

  // A leading `@Name` mention is direction, not prose: it renders as a
  // "to Name" line under the author instead of inside the message body.
  const directed = matchLeadingMention(event.content, mentionStyles);
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
        <span className="shrink-0 select-none text-xs text-muted-foreground/50">
          {formatTime(event.created_at)}
        </span>
        {reactions && reactions.length > 0 ? (
          <ReactionChips reactions={reactions} />
        ) : null}
      </div>
      {directed ? (
        <div className="select-none text-xs leading-4 text-muted-foreground/60">
          to{" "}
          <span style={{ color: directed.mention.color }}>
            {directed.mention.name}
          </span>
        </div>
      ) : null}
      <div
        className={cn(
          "min-w-0 whitespace-pre-wrap break-words [overflow-wrap:anywhere]",
          event.pending && "text-muted-foreground",
        )}
      >
        {renderHighlightedContent(bodyContent, mentionStyles, {
          channels,
          onOpen: openChannel,
        })}
      </div>
    </div>
  );
}
