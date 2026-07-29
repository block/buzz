import type { AuthorColorResolver } from "@/features/dev-mode/lib/authorColors";
import { renderHighlightedContent } from "@/features/dev-mode/lib/highlightContent";
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
  if (event.kind === KIND_SYSTEM_MESSAGE) {
    return null;
  }

  return (
    <div className="flex min-w-0 gap-2 py-0.5 text-sm leading-6">
      <span className="shrink-0 select-none text-muted-foreground/50">
        {formatTime(event.created_at)}
      </span>
      <span
        className={cn(
          "shrink-0 font-medium",
          isSelf && "underline decoration-dotted underline-offset-4",
        )}
        style={{ color: resolveColor(event.pubkey) }}
      >
        {resolveName(event.pubkey)}
      </span>
      <span
        className={cn(
          "min-w-0 flex-1 whitespace-pre-wrap break-words [overflow-wrap:anywhere]",
          event.pending && "text-muted-foreground",
        )}
      >
        {renderHighlightedContent(event.content)}
      </span>
      {reactions && reactions.length > 0 ? (
        <ReactionChips reactions={reactions} />
      ) : null}
    </div>
  );
}
