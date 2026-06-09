import { SmilePlus } from "lucide-react";
import * as React from "react";

import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import type { TimelineReaction } from "@/features/messages/types";
import { cn } from "@/shared/lib/cn";
import { emojiDisplayName } from "@/shared/lib/emojiName";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { AnimatedCount } from "@/shared/ui/AnimatedCount";
import {
  isPositiveEmojiParticle,
  useEmojiBurst,
} from "@/shared/ui/EmojiBurstProvider";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

const REACTION_PILL_BASE_CLASSES =
  "inline-flex h-8 items-center rounded-full border text-xs font-medium leading-none transition-colors";
const REACTION_GLYPH_CLASSES = "h-3.5 w-3.5 translate-y-px text-sm";
const REACTION_PILL_HOVER_CLASSES =
  "hover:bg-primary/10 hover:text-foreground focus-visible:bg-primary/10 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring";
const BADGE_BURST_STABLE_FRAMES = 2;
const BADGE_BURST_MAX_FRAMES = 12;
const BADGE_BURST_RECT_EPSILON = 0.5;

type BadgeBurstRect = {
  height: number;
  left: number;
  top: number;
  width: number;
};

function toBadgeBurstRect(rect: DOMRect): BadgeBurstRect {
  return {
    height: rect.height,
    left: rect.left,
    top: rect.top,
    width: rect.width,
  };
}

function isSameBadgeBurstRect(
  left: BadgeBurstRect | null,
  right: BadgeBurstRect,
) {
  return (
    left !== null &&
    Math.abs(left.left - right.left) <= BADGE_BURST_RECT_EPSILON &&
    Math.abs(left.top - right.top) <= BADGE_BURST_RECT_EPSILON &&
    Math.abs(left.width - right.width) <= BADGE_BURST_RECT_EPSILON &&
    Math.abs(left.height - right.height) <= BADGE_BURST_RECT_EPSILON
  );
}

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
  const displayName = emojiDisplayName(reaction.emoji);
  if (reaction.emojiUrl) {
    return (
      <img
        alt={reaction.emoji}
        title={displayName}
        src={rewriteRelayUrl(reaction.emojiUrl)}
        className={cn(
          "inline-block object-contain align-text-bottom",
          className,
        )}
        draggable={false}
      />
    );
  }
  return (
    <span
      className={cn(
        "inline-flex items-center justify-center leading-none",
        className,
      )}
      title={displayName}
    >
      {reaction.emoji}
    </span>
  );
}

function formatReactionUsers(reaction: TimelineReaction): string {
  const names = reaction.users.map((user) => user.displayName).filter(Boolean);
  if (reaction.reactedByCurrentUser) {
    const others = names.filter((name) => name !== "You");
    names.splice(0, names.length, "You (click to remove)", ...others);
  }
  if (names.length === 0) return `${reaction.count} people`;
  if (names.length === 1) return names[0];
  if (names.length === 2) return `${names[0]} and ${names[1]}`;
  return `${names.slice(0, -1).join(", ")}, and ${names[names.length - 1]}`;
}

function ReactionPopoverContent({ reaction }: { reaction: TimelineReaction }) {
  const displayName = emojiDisplayName(reaction.emoji);
  const userText = formatReactionUsers(reaction);

  return (
    <div className="flex flex-col items-center text-center">
      <div className="mb-2 flex h-14 w-14 items-center justify-center">
        <EmojiGlyph
          reaction={reaction}
          className={reaction.emojiUrl ? "h-12 w-12" : "text-4xl"}
        />
      </div>
      <div className="max-w-[14rem] text-balance text-sm font-semibold leading-snug text-popover-foreground">
        {userText} <span className="text-muted-foreground">reacted with</span>
      </div>
      <div className="mt-0.5 text-sm font-semibold leading-snug text-muted-foreground">
        {displayName}
      </div>
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
  burstEmojiOnRender = null,
  onBurstEmojiRendered,
}: {
  messageId: string;
  reactions: TimelineReaction[];
  canToggle: boolean;
  pending: boolean;
  onSelect: (emoji: string) => void;
  className?: string;
  burstEmojiOnRender?: string | null;
  onBurstEmojiRendered?: (emoji: string) => void;
}) {
  const { burstEmoji } = useEmojiBurst();
  const [pendingBadgeBurstEmoji, setPendingBadgeBurstEmoji] = React.useState<
    string | null
  >(null);
  const pillRefs = React.useRef(new Map<string, HTMLButtonElement>());
  const deliveredBadgeBurstRef = React.useRef<string | null>(null);
  const badgeBurstEmoji = burstEmojiOnRender ?? pendingBadgeBurstEmoji;

  const registerPill = React.useCallback(
    (emoji: string, element: HTMLButtonElement | null) => {
      if (element) {
        pillRefs.current.set(emoji, element);
      } else {
        pillRefs.current.delete(emoji);
      }
    },
    [],
  );

  React.useEffect(() => {
    if (!badgeBurstEmoji) {
      deliveredBadgeBurstRef.current = null;
      return;
    }
    if (
      deliveredBadgeBurstRef.current === badgeBurstEmoji ||
      !isPositiveEmojiParticle(badgeBurstEmoji)
    ) {
      return;
    }

    const reaction = reactions.find(
      (candidate) =>
        candidate.emoji === badgeBurstEmoji && candidate.reactedByCurrentUser,
    );
    if (!reaction || !pillRefs.current.get(badgeBurstEmoji)) return;

    let frameId: number | null = null;
    let frameCount = 0;
    let stableFrameCount = 0;
    let previousRect: BadgeBurstRect | null = null;

    const cancelFrame = () => {
      if (frameId === null) return;
      window.cancelAnimationFrame(frameId);
      frameId = null;
    };

    const emitFromSettledBadge = () => {
      frameId = null;

      const pill = pillRefs.current.get(badgeBurstEmoji);
      if (!pill || !document.documentElement.contains(pill)) return;

      const nextRect = toBadgeBurstRect(pill.getBoundingClientRect());
      if (!nextRect.width && !nextRect.height) return;

      stableFrameCount = isSameBadgeBurstRect(previousRect, nextRect)
        ? stableFrameCount + 1
        : 0;
      previousRect = nextRect;
      frameCount += 1;

      if (
        stableFrameCount < BADGE_BURST_STABLE_FRAMES &&
        frameCount < BADGE_BURST_MAX_FRAMES
      ) {
        frameId = window.requestAnimationFrame(emitFromSettledBadge);
        return;
      }

      deliveredBadgeBurstRef.current = badgeBurstEmoji;
      burstEmoji(badgeBurstEmoji, {
        clientX: nextRect.left + nextRect.width / 2,
        clientY: nextRect.top + nextRect.height / 2,
      });
      setPendingBadgeBurstEmoji((current) =>
        current === badgeBurstEmoji ? null : current,
      );
      onBurstEmojiRendered?.(badgeBurstEmoji);
    };

    frameId = window.requestAnimationFrame(emitFromSettledBadge);

    return cancelFrame;
  }, [badgeBurstEmoji, burstEmoji, onBurstEmojiRendered, reactions]);

  if (reactions.length === 0) {
    return null;
  }

  return (
    <div
      className={cn(
        "group/reactions mt-1.5 flex flex-wrap items-center gap-1.5 pt-1",
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
          registerPill={registerPill}
          onSelect={onSelect}
        />
      ))}
      {canToggle ? (
        <InlineReactionPicker
          messageId={messageId}
          onSelect={onSelect}
          pending={pending}
          reactions={reactions}
          requestBadgeBurst={setPendingBadgeBurstEmoji}
        />
      ) : null}
    </div>
  );
}

function InlineReactionPicker({
  messageId,
  onSelect,
  pending,
  reactions,
  requestBadgeBurst,
}: {
  messageId: string;
  onSelect: (emoji: string) => void;
  pending: boolean;
  reactions: TimelineReaction[];
  requestBadgeBurst: (emoji: string) => void;
}) {
  const [open, setOpen] = React.useState(false);
  const wouldAddReaction = (emoji: string) =>
    !reactions.some(
      (reaction) => reaction.emoji === emoji && reaction.reactedByCurrentUser,
    );

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
                  : "border-border/70 bg-muted/70",
                REACTION_PILL_HOVER_CLASSES,
              )}
              data-testid={`add-reaction-${messageId}`}
              disabled={pending}
              type="button"
            >
              <SmilePlus className="h-4 w-4" />
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
            if (wouldAddReaction(value) && isPositiveEmojiParticle(value)) {
              requestBadgeBurst(value);
            }
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
  registerPill,
  onSelect,
}: {
  reaction: TimelineReaction;
  canToggle: boolean;
  pending: boolean;
  registerPill: (emoji: string, element: HTMLButtonElement | null) => void;
  onSelect: (emoji: string) => void;
}) {
  const { burstEmoji } = useEmojiBurst();
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

  const setPillRef = React.useCallback(
    (element: HTMLButtonElement | null) => {
      registerPill(reaction.emoji, element);
    },
    [reaction.emoji, registerPill],
  );

  const pillClasses = cn(
    REACTION_PILL_BASE_CLASSES,
    "min-w-12 justify-center gap-1.5 px-2",
    reaction.reactedByCurrentUser
      ? "border-primary/40 bg-primary/10 text-primary"
      : "border-border/70 bg-muted/70 text-foreground/90",
    canToggle
      ? reaction.reactedByCurrentUser
        ? "hover:bg-primary/10 hover:text-primary focus-visible:bg-primary/10 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        : REACTION_PILL_HOVER_CLASSES
      : "cursor-default",
  );

  const handleClick = (event: React.MouseEvent<HTMLButtonElement>) => {
    if (!canToggle) return;
    if (
      !reaction.reactedByCurrentUser &&
      isPositiveEmojiParticle(reaction.emoji)
    ) {
      burstEmoji(reaction.emoji, event);
    }
    onSelect(reaction.emoji);
  };

  const displayName = emojiDisplayName(reaction.emoji);

  if (reaction.users.length === 0) {
    return (
      <button
        aria-label={`Toggle ${reaction.emoji} reaction`}
        aria-pressed={reaction.reactedByCurrentUser}
        title={displayName}
        className={pillClasses}
        disabled={!canToggle || pending}
        onClick={handleClick}
        ref={setPillRef}
        type="button"
      >
        <EmojiGlyph reaction={reaction} className={REACTION_GLYPH_CLASSES} />
        <AnimatedCount
          className="text-muted-foreground"
          value={reaction.count}
        />
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
            title={displayName}
            className={pillClasses}
            disabled={!canToggle || pending}
            onClick={handleClick}
            ref={setPillRef}
            type="button"
          >
            <EmojiGlyph
              reaction={reaction}
              className={REACTION_GLYPH_CLASSES}
            />
            <AnimatedCount
              className="text-muted-foreground"
              value={reaction.count}
            />
          </button>
        </span>
      </PopoverTrigger>
      <PopoverContent
        align="start"
        side="top"
        sideOffset={6}
        className="w-auto min-w-56 max-w-72 rounded-xl p-3"
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
