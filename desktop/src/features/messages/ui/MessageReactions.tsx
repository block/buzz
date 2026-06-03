import * as React from "react";

import type { TimelineReaction } from "@/features/messages/types";
import { cn } from "@/shared/lib/cn";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const MAX_VISIBLE_REACTORS = 10;

/**
 * Render a reaction's emoji: a custom (image) emoji when `emojiUrl` is set,
 * otherwise the unicode/text glyph. `className` sizes the image to match the
 * surrounding text. The relay URL is rewritten through the localhost media
 * proxy (like every other relay-hosted <img>) — WKWebView bypasses WARP, so a
 * direct relay URL gets a Cloudflare Access 403 and renders as a broken image.
 */
function EmojiGlyph({
  reaction,
  className,
}: {
  reaction: TimelineReaction;
  className?: string;
}) {
  if (reaction.emojiUrl) {
    return (
      <img
        alt={reaction.emoji}
        src={rewriteRelayUrl(reaction.emojiUrl)}
        className={cn(
          "inline-block object-contain align-text-bottom",
          className,
        )}
        draggable={false}
      />
    );
  }
  return <span>{reaction.emoji}</span>;
}

function ReactionPopoverContent({ reaction }: { reaction: TimelineReaction }) {
  const visible = reaction.users.slice(0, MAX_VISIBLE_REACTORS);
  const overflow = reaction.users.length - MAX_VISIBLE_REACTORS;

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2 pb-1 border-b border-border/50">
        <EmojiGlyph reaction={reaction} className="h-6 w-6 text-2xl" />
        <span className="text-xs text-muted-foreground">
          {reaction.count} {reaction.count === 1 ? "reaction" : "reactions"}
        </span>
      </div>
      <div className="flex flex-col gap-1.5">
        {visible.map((user) => (
          <div key={user.pubkey} className="flex items-center gap-2 min-w-0">
            <UserAvatar
              avatarUrl={user.avatarUrl}
              displayName={user.displayName}
              size="xs"
            />
            <span className="text-sm truncate">{user.displayName}</span>
          </div>
        ))}
      </div>
      {overflow > 0 && (
        <span className="text-xs text-muted-foreground">+{overflow} more</span>
      )}
      {reaction.reactedByCurrentUser && (
        <span className="text-xs text-muted-foreground border-t border-border/50 pt-1.5">
          Click to remove your reaction
        </span>
      )}
    </div>
  );
}

export function MessageReactions({
  messageId,
  reactions,
  canToggle,
  pending,
  onSelect,
  className,
}: {
  messageId: string;
  reactions: TimelineReaction[];
  canToggle: boolean;
  pending: boolean;
  onSelect: (emoji: string) => void;
  className?: string;
}) {
  if (reactions.length === 0) {
    return null;
  }

  return (
    <div
      className={cn(
        "mt-1.5 flex flex-wrap items-center gap-1.5 pt-1",
        className,
      )}
    >
      {reactions.map((reaction) => (
        <ReactionPill
          key={`${messageId}-${reaction.emoji}`}
          canToggle={canToggle}
          pending={pending}
          reaction={reaction}
          onSelect={onSelect}
        />
      ))}
    </div>
  );
}

function ReactionPill({
  reaction,
  canToggle,
  pending,
  onSelect,
}: {
  reaction: TimelineReaction;
  canToggle: boolean;
  pending: boolean;
  onSelect: (emoji: string) => void;
}) {
  const [open, setOpen] = React.useState(false);
  const openTimeout = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const closeTimeout = React.useRef<ReturnType<typeof setTimeout> | null>(null);

  const clearTimers = React.useCallback(() => {
    if (openTimeout.current) {
      clearTimeout(openTimeout.current);
      openTimeout.current = null;
    }
    if (closeTimeout.current) {
      clearTimeout(closeTimeout.current);
      closeTimeout.current = null;
    }
  }, []);

  const handleMouseEnter = React.useCallback(() => {
    if (reaction.users.length === 0) return;
    clearTimers();
    openTimeout.current = setTimeout(() => setOpen(true), 200);
  }, [reaction.users.length, clearTimers]);

  const scheduleClose = React.useCallback(() => {
    clearTimers();
    closeTimeout.current = setTimeout(() => setOpen(false), 150);
  }, [clearTimers]);

  const handleFocus = React.useCallback(() => {
    if (reaction.users.length === 0) return;
    clearTimers();
    setOpen(true);
  }, [reaction.users.length, clearTimers]);

  React.useEffect(() => {
    return clearTimers;
  }, [clearTimers]);

  const pillClasses = cn(
    "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs font-medium transition-colors",
    reaction.reactedByCurrentUser
      ? "border-primary/40 bg-primary/10 text-primary"
      : "border-border/70 bg-muted/70 text-foreground/90",
    canToggle
      ? "hover:bg-accent hover:text-accent-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
      : "cursor-default",
  );

  const handleClick = () => {
    if (!canToggle) return;
    onSelect(reaction.emoji);
  };

  if (reaction.users.length === 0) {
    return (
      <button
        aria-label={`Toggle ${reaction.emoji} reaction`}
        aria-pressed={reaction.reactedByCurrentUser}
        className={pillClasses}
        disabled={!canToggle || pending}
        onClick={handleClick}
        type="button"
      >
        <EmojiGlyph reaction={reaction} className="h-[1.1em] w-[1.1em]" />
        <span className="text-muted-foreground">{reaction.count}</span>
      </button>
    );
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        {/* biome-ignore lint/a11y/noStaticElementInteractions: span delegates hover/focus to disabled button */}
        <span
          className="inline-flex"
          onMouseEnter={handleMouseEnter}
          onMouseLeave={scheduleClose}
          onFocus={handleFocus}
          onBlur={scheduleClose}
        >
          <button
            aria-label={`Toggle ${reaction.emoji} reaction`}
            aria-pressed={reaction.reactedByCurrentUser}
            className={pillClasses}
            disabled={!canToggle || pending}
            onClick={handleClick}
            type="button"
          >
            <EmojiGlyph reaction={reaction} className="h-[1.1em] w-[1.1em]" />
            <span className="text-muted-foreground">{reaction.count}</span>
          </button>
        </span>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        side="top"
        sideOffset={6}
        className="w-auto min-w-48 max-w-64 p-3"
        onMouseEnter={handleMouseEnter}
        onMouseLeave={scheduleClose}
        onOpenAutoFocus={(e) => e.preventDefault()}
        onCloseAutoFocus={(e) => e.preventDefault()}
      >
        <ReactionPopoverContent reaction={reaction} />
      </PopoverContent>
    </Popover>
  );
}
