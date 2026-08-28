import { CornerUpLeft, EllipsisVertical, Link2, SmilePlus } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import * as React from "react";

import { useCustomEmoji } from "@/features/custom-emoji/hooks";
import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import { reactionEmojiUrl } from "@/shared/api/customEmoji";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { isPositiveEmojiParticle } from "@/shared/ui/EmojiBurstProvider";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import {
  BestieMessagePanel,
  useBestieMessageAgent,
} from "@/features/messages/ui/BestieMessagePopover";
import type { MessageActionBarProps } from "@/features/messages/ui/MessageActionBar";
import {
  MESSAGE_ACTION_BLOOM_EASE_OUT,
  MESSAGE_ACTION_BLOOM_SPEED_MULTIPLIER,
  MESSAGE_ACTION_BLOOM_VISUAL_DURATION,
  MessageActionBloomSurface,
} from "@/features/messages/ui/MessageActionBloomSurface";
import { MessageActionOverflowPanel } from "@/features/messages/ui/MessageActionOverflowPanel";
import {
  canCopyMessageLink,
  copyMessageLink,
  isCustomEmojiShortcode,
  QuickReactionButton,
} from "@/features/messages/ui/MessageActionToolbarHelpers";
import {
  recordQuickReactionEmoji,
  useQuickReactionEmojis,
} from "@/features/messages/ui/useQuickReactionEmojis";

const ACTION_BUTTON_CLASS = "h-8 w-8 rounded-full p-0";
const ACTION_ICON_CLASS = "!h-4 !w-4";
type ActiveSurface = "reactions" | "bestie" | "more" | null;

export function MessageActionBloomBar({
  channelId,
  message,
  onDelete,
  onEdit,
  onExpandedChange,
  onFollowThread,
  onMarkUnread,
  onMarkRead,
  onReactionBadgeBurstRequest,
  onReactionSelect,
  onRemindLater,
  onReply,
  onSendToChannel,
  onUnfollowThread,
  reactionErrorMessage = null,
  reactions,
  isFollowingThread,
  isUnread,
}: MessageActionBarProps) {
  const [activeSurface, setActiveSurface] = React.useState<ActiveSurface>(null);
  const reduceMotion = useReducedMotion();
  const surfaceRef = React.useRef<HTMLDivElement>(null);
  const toolbarRef = React.useRef<HTMLDivElement>(null);
  const activeContentRef = React.useRef<HTMLDivElement>(null);
  const anchorRectRef = React.useRef<DOMRect | null>(null);
  const demotionTimerRef = React.useRef<number | null>(null);
  const [closedSize, setClosedSize] = React.useState<{
    height: number;
    width: number;
  } | null>(null);
  const [openSize, setOpenSize] = React.useState<{
    height: number;
    width: number;
  } | null>(null);
  const [contentReady, setContentReady] = React.useState(false);
  const [expansionDirection, setExpansionDirection] = React.useState<
    "up" | "down"
  >("up");
  const reactionTriggerRef = React.useRef<HTMLButtonElement>(null);
  const bestieTriggerRef = React.useRef<HTMLButtonElement>(null);
  const moreTriggerRef = React.useRef<HTMLButtonElement>(null);
  const lastSurfaceRef = React.useRef<ActiveSurface>(null);
  const customEmoji = useCustomEmoji();
  const bestie = useBestieMessageAgent();
  const quickReactionEmojis = useQuickReactionEmojis(3, customEmoji);
  const quickReactionItems = React.useMemo(
    () =>
      quickReactionEmojis
        .map((emoji) => ({
          customEmojiUrl: reactionEmojiUrl(emoji, customEmoji),
          emoji,
        }))
        .filter(
          (item) => !isCustomEmojiShortcode(item.emoji) || item.customEmojiUrl,
        ),
    [customEmoji, quickReactionEmojis],
  );
  const hasReplyAction = Boolean(onReply);
  const hasReactionAction = Boolean(onReactionSelect);
  const hasMoreMenuActions =
    Boolean(onEdit) ||
    Boolean(onDelete) ||
    Boolean(onMarkUnread) ||
    Boolean(onMarkRead) ||
    Boolean(onFollowThread) ||
    Boolean(onUnfollowThread) ||
    Boolean(onRemindLater) ||
    Boolean(onSendToChannel) ||
    !message.pending;

  React.useEffect(() => {
    onExpandedChange?.(activeSurface !== null);
    return () => onExpandedChange?.(false);
  }, [activeSurface, onExpandedChange]);

  const wouldAddReaction = React.useCallback(
    (emoji: string) =>
      !reactions.some(
        (reaction) => reaction.emoji === emoji && reaction.reactedByCurrentUser,
      ),
    [reactions],
  );
  const handleReactionSelection = React.useCallback(
    (emoji: string, closePicker = false) => {
      if (!onReactionSelect) return;
      if (wouldAddReaction(emoji) && isPositiveEmojiParticle(emoji)) {
        onReactionBadgeBurstRequest?.(emoji);
      }
      void onReactionSelect(emoji)
        .then(() => recordQuickReactionEmoji(emoji))
        .catch(() => {})
        .finally(() => {
          if (closePicker) setActiveSurface(null);
        });
    },
    [onReactionBadgeBurstRequest, onReactionSelect, wouldAddReaction],
  );

  const closeSurface = React.useCallback((restoreFocus = false) => {
    const surface = lastSurfaceRef.current;
    setActiveSurface(null);
    if (!restoreFocus) return;
    window.requestAnimationFrame(() => {
      if (surface === "reactions") reactionTriggerRef.current?.focus();
      if (surface === "bestie") bestieTriggerRef.current?.focus();
      if (surface === "more") moreTriggerRef.current?.focus();
    });
  }, []);

  const openSurface = React.useCallback(
    (surface: Exclude<ActiveSurface, null>) => {
      const bloom = surfaceRef.current;
      if (bloom && !bloom.matches(":popover-open")) {
        const anchorRect = bloom.getBoundingClientRect();
        anchorRectRef.current = anchorRect;
        bloom.setAttribute("popover", "manual");
        bloom.style.position = "fixed";
        bloom.style.inset = "auto";
        bloom.style.right = `${window.innerWidth - anchorRect.right}px`;
        bloom.style.bottom = `${window.innerHeight - anchorRect.bottom}px`;
        bloom.style.margin = "0";
        bloom.showPopover();
      }
      if (demotionTimerRef.current !== null) {
        window.clearTimeout(demotionTimerRef.current);
        demotionTimerRef.current = null;
      }
      lastSurfaceRef.current = surface;
      setContentReady(false);
      setOpenSize(null);
      setActiveSurface(surface);
    },
    [],
  );

  React.useLayoutEffect(() => {
    if (!activeSurface) return;
    const bloom = surfaceRef.current;
    const anchorRect = anchorRectRef.current;
    if (!bloom || !anchorRect) return;
    bloom.style.right = `${window.innerWidth - anchorRect.right}px`;
    if (expansionDirection === "up") {
      bloom.style.top = "auto";
      bloom.style.bottom = `${window.innerHeight - anchorRect.bottom}px`;
    } else {
      bloom.style.top = `${anchorRect.top}px`;
      bloom.style.bottom = "auto";
    }
  }, [activeSurface, expansionDirection]);

  React.useEffect(() => {
    if (activeSurface) return;
    const bloom = surfaceRef.current;
    if (!bloom?.matches(":popover-open")) return;
    demotionTimerRef.current = window.setTimeout(
      () => {
        if (bloom.matches(":popover-open")) bloom.hidePopover();
        bloom.removeAttribute("popover");
        for (const property of [
          "position",
          "inset",
          "right",
          "bottom",
          "top",
          "margin",
        ]) {
          bloom.style.removeProperty(property);
        }
        anchorRectRef.current = null;
        demotionTimerRef.current = null;
      },
      reduceMotion
        ? 0
        : MESSAGE_ACTION_BLOOM_VISUAL_DURATION * 1000 +
            20 / MESSAGE_ACTION_BLOOM_SPEED_MULTIPLIER,
    );
    return () => {
      if (demotionTimerRef.current !== null) {
        window.clearTimeout(demotionTimerRef.current);
        demotionTimerRef.current = null;
      }
    };
  }, [activeSurface, reduceMotion]);

  React.useEffect(
    () => () => {
      if (demotionTimerRef.current !== null) {
        window.clearTimeout(demotionTimerRef.current);
      }
    },
    [],
  );

  React.useLayoutEffect(() => {
    const toolbar = toolbarRef.current;
    if (!toolbar) return;
    const measure = () => {
      const rect = toolbar.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return;
      setClosedSize((current) =>
        current?.height === rect.height && current.width === rect.width
          ? current
          : { height: rect.height, width: rect.width },
      );
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(toolbar);
    return () => observer.disconnect();
  }, []);

  React.useLayoutEffect(() => {
    if (!activeSurface) return;
    const content = activeContentRef.current;
    if (!content) return;
    let firstFrame = 0;
    let settledFrame = 0;
    const measure = () => {
      window.cancelAnimationFrame(firstFrame);
      window.cancelAnimationFrame(settledFrame);
      firstFrame = window.requestAnimationFrame(() => {
        settledFrame = window.requestAnimationFrame(() => {
          const rect = content.getBoundingClientRect();
          if (rect.width <= 0 || rect.height <= 0) return;
          const surfaceRect = surfaceRef.current?.getBoundingClientRect();
          if (surfaceRect) {
            const roomAbove = surfaceRect.bottom - 64;
            const roomBelow = window.innerHeight - surfaceRect.top - 64;
            setExpansionDirection(
              rect.height <= roomAbove || roomAbove >= roomBelow
                ? "up"
                : "down",
            );
          }
          const nextSize = {
            height: Math.ceil(rect.height),
            width: Math.ceil(rect.width),
          };
          setOpenSize((current) =>
            current?.height === nextSize.height &&
            current.width === nextSize.width
              ? current
              : nextSize,
          );
          setContentReady(true);
        });
      });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(content);
    return () => {
      observer.disconnect();
      window.cancelAnimationFrame(firstFrame);
      window.cancelAnimationFrame(settledFrame);
    };
  }, [activeSurface]);

  React.useEffect(() => {
    if (!activeSurface) return;
    const currentBar = surfaceRef.current?.closest("[data-message-action-bar]");
    const otherBars = Array.from(
      document.querySelectorAll<HTMLElement>("[data-message-action-bar]"),
    ).filter((bar) => bar !== currentBar);
    for (const bar of otherBars) bar.inert = true;
    const handlePointerDown = (event: PointerEvent) => {
      if (event.composedPath().includes(surfaceRef.current as EventTarget))
        return;
      const target = event.target;
      if (
        target instanceof Element &&
        target.closest('[data-radix-popper-content-wrapper], [role="dialog"]')
      ) {
        return;
      }
      closeSurface();
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      closeSurface(true);
    };
    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown, true);
    return () => {
      for (const bar of otherBars) bar.inert = false;
      document.removeEventListener("pointerdown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown, true);
    };
  }, [activeSurface, closeSurface]);

  if (!hasReplyAction && !hasReactionAction && !hasMoreMenuActions) return null;

  const panelMotion = {
    animate: {
      filter: contentReady ? "blur(0px)" : "blur(6px)",
      opacity: contentReady ? 1 : 0,
      y: contentReady ? 0 : expansionDirection === "up" ? 8 : -8,
    },
    exit: {
      filter: "blur(6px)",
      opacity: 0,
      y: expansionDirection === "up" ? 8 : -8,
    },
    transition: {
      delay:
        reduceMotion || !contentReady
          ? 0
          : 0.01 / MESSAGE_ACTION_BLOOM_SPEED_MULTIPLIER,
      duration: reduceMotion ? 0 : 0.1 / MESSAGE_ACTION_BLOOM_SPEED_MULTIPLIER,
      ease: MESSAGE_ACTION_BLOOM_EASE_OUT,
    },
  } as const;

  return (
    <div
      className={cn(
        "relative h-10 w-0 transition-opacity duration-150 ease-out",
        "opacity-100 sm:pointer-events-none sm:opacity-0",
        "sm:group-hover/message:pointer-events-auto sm:group-hover/message:opacity-100",
        "sm:group-focus-within/message:pointer-events-auto sm:group-focus-within/message:opacity-100",
        activeSurface ? "sm:pointer-events-auto sm:opacity-100" : "",
      )}
      data-bloom-surface={activeSurface ?? "toolbar"}
      data-message-action-bar
      data-testid={`message-action-bar-${message.id}`}
    >
      <MessageActionBloomSurface
        className={cn(
          "absolute right-0 m-0 p-0 [&::backdrop]:hidden",
          expansionDirection === "up" ? "bottom-0" : "top-0",
        )}
        data-testid={`message-action-bloom-container-${message.id}`}
        expanded={activeSurface !== null && contentReady}
        ref={surfaceRef}
        size={activeSurface !== null && contentReady ? openSize : closedSize}
      >
        <motion.div
          animate={{ opacity: activeSurface && contentReady ? 0 : 1 }}
          className="flex w-max shrink-0 flex-nowrap items-center gap-0.5 p-1"
          data-testid={`message-action-bloom-surface-${message.id}`}
          initial={false}
          ref={toolbarRef}
          style={{ pointerEvents: activeSurface === null ? "auto" : "none" }}
          transition={{ duration: reduceMotion ? 0 : 0.1 }}
        >
          {hasReactionAction && quickReactionItems.length > 0 ? (
            <div className="hidden items-center gap-0.5 sm:flex">
              {quickReactionItems.map(({ customEmojiUrl, emoji }) => (
                <QuickReactionButton
                  customEmojiUrl={customEmojiUrl}
                  emoji={emoji}
                  key={emoji}
                  onSelect={handleReactionSelection}
                />
              ))}
            </div>
          ) : null}
          {hasReactionAction ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  aria-label="Open reactions"
                  className={ACTION_BUTTON_CLASS}
                  data-testid={`react-message-${message.id}`}
                  onClick={() => openSurface("reactions")}
                  ref={reactionTriggerRef}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  <SmilePlus className={ACTION_ICON_CLASS} />
                </Button>
              </TooltipTrigger>
              <TooltipContent>React</TooltipContent>
            </Tooltip>
          ) : null}
          {hasReactionAction && quickReactionItems.length > 0 ? (
            <div
              aria-hidden="true"
              className="mx-0.5 hidden h-4 w-px bg-border/70 sm:block"
              data-testid="message-action-divider"
            />
          ) : null}
          {hasReplyAction ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  aria-label="Reply"
                  className={ACTION_BUTTON_CLASS}
                  data-testid={`reply-message-${message.id}`}
                  onClick={() => onReply?.(message)}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  <CornerUpLeft className={ACTION_ICON_CLASS} />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Reply</TooltipContent>
            </Tooltip>
          ) : null}
          {bestie && canCopyMessageLink(message, channelId) ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  aria-label="Send to Bestie"
                  className={ACTION_BUTTON_CLASS}
                  data-testid={`send-to-bestie-${message.id}`}
                  onClick={() => openSurface("bestie")}
                  ref={bestieTriggerRef}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  <ProfileAvatar
                    avatarUrl={bestie.avatarUrl}
                    className="size-5 text-3xs"
                    label={bestie.name}
                    testId={`bestie-action-avatar-${message.id}`}
                  />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Bestie</TooltipContent>
            </Tooltip>
          ) : null}
          {canCopyMessageLink(message, channelId) ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  aria-label="Copy link"
                  className={ACTION_BUTTON_CLASS}
                  data-testid={`copy-link-message-${message.id}`}
                  onClick={() => copyMessageLink(channelId, message)}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  <Link2 className={ACTION_ICON_CLASS} />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Copy link</TooltipContent>
            </Tooltip>
          ) : null}
          {hasMoreMenuActions ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  aria-label="More actions"
                  className={ACTION_BUTTON_CLASS}
                  data-testid={`more-actions-${message.id}`}
                  onClick={() => openSurface("more")}
                  ref={moreTriggerRef}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  <EllipsisVertical className={ACTION_ICON_CLASS} />
                </Button>
              </TooltipTrigger>
              <TooltipContent>More actions</TooltipContent>
            </Tooltip>
          ) : null}
        </motion.div>

        <AnimatePresence initial={false}>
          {activeSurface === "reactions" ? (
            <motion.div
              {...panelMotion}
              className={cn(
                "absolute right-0",
                expansionDirection === "up" ? "bottom-0" : "top-0",
              )}
              data-testid={`reaction-bloom-panel-${message.id}`}
              initial={false}
              key="reactions"
              ref={activeContentRef}
              style={{ pointerEvents: contentReady ? "auto" : "none" }}
            >
              {reactionErrorMessage ? (
                <div className="px-3 pb-0 pt-3">
                  <p className="text-xs text-destructive">
                    {reactionErrorMessage}
                  </p>
                </div>
              ) : null}
              <EmojiPicker
                autoFocus
                onSelect={(value) => handleReactionSelection(value, true)}
              />
            </motion.div>
          ) : null}
          {activeSurface === "more" ? (
            <motion.div
              {...panelMotion}
              className={cn(
                "absolute right-0",
                expansionDirection === "up" ? "bottom-0" : "top-0",
              )}
              initial={false}
              key="more"
              ref={activeContentRef}
              style={{ pointerEvents: contentReady ? "auto" : "none" }}
            >
              <MessageActionOverflowPanel
                channelId={channelId}
                isFollowingThread={isFollowingThread}
                isUnread={isUnread}
                message={message}
                onClose={() => closeSurface()}
                onDelete={onDelete}
                onEdit={onEdit}
                onFollowThread={onFollowThread}
                onMarkRead={onMarkRead}
                onMarkUnread={onMarkUnread}
                onRemindLater={onRemindLater}
                onSendToChannel={onSendToChannel}
                onUnfollowThread={onUnfollowThread}
              />
            </motion.div>
          ) : null}
          {activeSurface === "bestie" && bestie && channelId ? (
            <motion.div
              {...panelMotion}
              className={cn(
                "absolute right-0",
                expansionDirection === "up" ? "bottom-0" : "top-0",
              )}
              initial={false}
              key="bestie"
              ref={activeContentRef}
              style={{ pointerEvents: contentReady ? "auto" : "none" }}
            >
              <BestieMessagePanel
                bestie={bestie}
                channelId={channelId}
                message={message}
                onClose={() => closeSurface()}
              />
            </motion.div>
          ) : null}
        </AnimatePresence>
      </MessageActionBloomSurface>
    </div>
  );
}
