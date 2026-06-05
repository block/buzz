import { SmilePlus } from "lucide-react";
import * as React from "react";

import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import type { TimelineReaction } from "@/features/messages/types";
import { cn } from "@/shared/lib/cn";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Spinner } from "@/shared/ui/spinner";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const MAX_VISIBLE_REACTORS = 10;
const REACTION_PILL_BASE_CLASSES =
  "inline-flex h-[26px] items-center rounded-full border text-xs font-medium leading-none transition-colors";
const REACTION_GLYPH_CLASSES = "-ml-0.5 h-[26px] w-3 text-xs";
const REACTION_PILL_HOVER_CLASSES =
  "hover:bg-primary/10 hover:text-foreground focus-visible:bg-primary/10 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring";

/**
 * Render a reaction's emoji in a fixed centering frame. Unicode emoji fonts
 * have uneven visual bounds, so the outer frame does the vertical alignment
 * instead of relying on line-height.
 */
function EmojiGlyph({
  reaction,
  className,
}: {
  reaction: TimelineReaction;
  className?: string;
}) {
  const frameClassName = cn(
    "inline-grid shrink-0 place-items-center leading-none",
    className,
  );

  if (reaction.emojiUrl) {
    return (
      <span className={frameClassName}>
        <img
          alt={reaction.emoji}
          src={rewriteRelayUrl(reaction.emojiUrl)}
          className="block h-full w-full object-contain"
          draggable={false}
        />
      </span>
    );
  }
  return (
    <span className={frameClassName}>
      <span className="block leading-none">{reaction.emoji}</span>
    </span>
  );
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
        "group/reactions mt-1 flex flex-wrap items-center gap-1.5 pt-1",
        className,
      )}
      data-testid="message-reactions"
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
      {canToggle ? (
        <InlineReactionPicker
          messageId={messageId}
          onSelect={onSelect}
          pending={pending}
        />
      ) : null}
    </div>
  );
}

function InlineReactionPicker({
  messageId,
  onSelect,
  pending,
}: {
  messageId: string;
  onSelect: (emoji: string) => void;
  pending: boolean;
}) {
  const [open, setOpen] = React.useState(false);

  return (
    <Popover onOpenChange={setOpen} open={open}>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <button
              aria-label="Add reaction"
              className={cn(
                REACTION_PILL_BASE_CLASSES,
                "pointer-events-none w-10 min-w-10 justify-center p-0 text-muted-foreground opacity-0",
                "group-hover/message:pointer-events-auto group-hover/message:opacity-100",
                "group-focus-within/message:pointer-events-auto group-focus-within/message:opacity-100",
                "group-hover/reactions:pointer-events-auto group-hover/reactions:opacity-100",
                "group-focus-within/reactions:pointer-events-auto group-focus-within/reactions:opacity-100",
                open
                  ? "pointer-events-auto border-border/80 bg-background text-foreground opacity-100 shadow-xs"
                  : "border-transparent bg-muted/70",
                REACTION_PILL_HOVER_CLASSES,
              )}
              data-testid={`add-reaction-${messageId}`}
              disabled={pending}
              type="button"
            >
              {pending ? (
                <Spinner className="h-4 w-4" />
              ) : (
                <SmilePlus className="h-4 w-4" />
              )}
            </button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent>React</TooltipContent>
      </Tooltip>
      <PopoverContent
        align="start"
        className="w-auto overflow-hidden rounded-2xl border-0 bg-transparent p-0 shadow-none"
        side="top"
        sideOffset={8}
      >
        <EmojiPicker
          autoFocus
          onSelect={(value) => {
            onSelect(value);
            setOpen(false);
          }}
        />
      </PopoverContent>
    </Popover>
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
    REACTION_PILL_BASE_CLASSES,
    "min-w-12 justify-center gap-2 px-2",
    reaction.reactedByCurrentUser
      ? "border-primary/40 bg-primary/10 text-primary"
      : "border-transparent bg-muted/70 text-foreground/90",
    canToggle
      ? reaction.reactedByCurrentUser
        ? "hover:bg-primary/10 hover:text-primary focus-visible:bg-primary/10 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        : REACTION_PILL_HOVER_CLASSES
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
        <EmojiGlyph reaction={reaction} className={REACTION_GLYPH_CLASSES} />
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
            <EmojiGlyph
              reaction={reaction}
              className={REACTION_GLYPH_CLASSES}
            />
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
