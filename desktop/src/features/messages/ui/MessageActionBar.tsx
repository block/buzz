import {
  BellOff,
  BellRing,
  Clock,
  Copy,
  CornerUpLeft,
  EllipsisVertical,
  Link2,
  MailOpen,
  Pencil,
  SmilePlus,
  Trash2,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { buildMessageLink } from "@/features/messages/lib/messageLink";
import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import { useCustomEmoji } from "@/features/custom-emoji/hooks";
import { getThreadReference } from "@/features/messages/lib/threading";
import type {
  TimelineMessage,
  TimelineReaction,
} from "@/features/messages/types";
import {
  recordQuickReactionEmoji,
  useQuickReactionEmojis,
} from "@/features/messages/ui/useQuickReactionEmojis";
import { reactionEmojiUrl } from "@/shared/api/customEmoji";
import { cn } from "@/shared/lib/cn";
import { emojiDisplayName } from "@/shared/lib/emojiName";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { isPositiveEmojiParticle } from "@/shared/ui/EmojiBurstProvider";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

const ACTION_BUTTON_CLASS = "h-8 w-8 rounded-full p-0";
const ACTION_ICON_CLASS = "!h-4 !w-4";

function copyToClipboard(text: string, successMessage: string) {
  void navigator.clipboard
    .writeText(text)
    .then(() => {
      toast.success(successMessage);
    })
    .catch(() => {
      toast.error("Failed to copy to clipboard");
    });
}

// ---------------------------------------------------------------------------
// MoreActionsMenu — dropdown with edit, mark unread, copy, and delete actions
// ---------------------------------------------------------------------------

function MoreActionsMenu({
  channelId,
  message,
  onDelete,
  onEdit,
  onFollowThread,
  onMarkUnread,
  onOpenChange,
  onRemindLater,
  onUnfollowThread,
  open,
  isFollowingThread,
}: {
  /** Channel UUID for the "Copy link" action. When null/undefined, the
   *  Copy link entry is hidden (e.g. inbox preview rows that don't have it). */
  channelId?: string | null;
  message: TimelineMessage;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onFollowThread?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onOpenChange: (open: boolean) => void;
  onRemindLater?: (message: TimelineMessage) => void;
  onUnfollowThread?: (message: TimelineMessage) => void;
  open: boolean;
  isFollowingThread?: boolean;
}) {
  const [isDeleteDialogOpen, setIsDeleteDialogOpen] = React.useState(false);
  // Set true the moment the user picks "Edit message". The
  // `onCloseAutoFocus` handler on `DropdownMenuContent` reads it to
  // suppress Radix's default focus-restoration (which would yank focus
  // back to the trigger and steal it from the composer's editor — the
  // composer schedules its own focus on RAF, but Radix's restoration
  // runs in a setTimeout that fires after our RAF and wins the race).
  // Reset to false inside the handler so Escape / non-Edit closes still
  // get default trigger-restoration (a11y intact for keyboard users).
  const editJustSelectedRef = React.useRef(false);

  const hasCopyActions = !message.pending;

  return (
    <>
      <DropdownMenu open={open} onOpenChange={onOpenChange}>
        <Tooltip>
          <TooltipTrigger asChild>
            <DropdownMenuTrigger asChild>
              <Button
                aria-label="More actions"
                className={ACTION_BUTTON_CLASS}
                data-testid={`more-actions-${message.id}`}
                size="sm"
                type="button"
                variant={open ? "secondary" : "ghost"}
              >
                <EllipsisVertical className={ACTION_ICON_CLASS} />
              </Button>
            </DropdownMenuTrigger>
          </TooltipTrigger>
          <TooltipContent>More actions</TooltipContent>
        </Tooltip>
        <DropdownMenuContent
          align="end"
          side="top"
          sideOffset={6}
          onCloseAutoFocus={(event) => {
            if (editJustSelectedRef.current) {
              event.preventDefault();
              editJustSelectedRef.current = false;
            }
          }}
        >
          {onEdit ? (
            <DropdownMenuItem
              data-testid={`edit-message-${message.id}`}
              onClick={() => {
                editJustSelectedRef.current = true;
                onEdit(message);
              }}
            >
              <Pencil className="h-4 w-4" />
              Edit message
            </DropdownMenuItem>
          ) : null}

          {onMarkUnread ? (
            <DropdownMenuItem
              onClick={() => {
                onMarkUnread(message);
              }}
            >
              <MailOpen className="h-4 w-4" />
              Mark unread
            </DropdownMenuItem>
          ) : null}

          {onFollowThread || onUnfollowThread ? (
            <DropdownMenuItem
              onClick={() => {
                if (isFollowingThread) {
                  onUnfollowThread?.(message);
                } else {
                  onFollowThread?.(message);
                }
              }}
            >
              {isFollowingThread ? (
                <BellOff className="h-4 w-4" />
              ) : (
                <BellRing className="h-4 w-4" />
              )}
              {isFollowingThread ? "Unfollow thread" : "Follow thread"}
            </DropdownMenuItem>
          ) : null}

          {hasCopyActions ? (
            <DropdownMenuItem
              onClick={() => {
                copyToClipboard(message.body, "Message copied to clipboard");
              }}
            >
              <Copy className="h-4 w-4" />
              Copy message
            </DropdownMenuItem>
          ) : null}

          {onRemindLater ? (
            <DropdownMenuItem
              onClick={() => {
                onRemindLater(message);
              }}
            >
              <Clock className="h-4 w-4" />
              Remind me later
            </DropdownMenuItem>
          ) : null}

          {hasCopyActions && channelId ? (
            <DropdownMenuItem
              data-testid={`copy-message-link-${message.id}`}
              onClick={() => {
                const { rootId } = getThreadReference(message.tags ?? []);
                const link = buildMessageLink({
                  channelId,
                  messageId: message.id,
                  threadRootId: rootId,
                });
                copyToClipboard(link, "Link copied to clipboard");
              }}
            >
              <Link2 className="h-4 w-4" />
              Copy link
            </DropdownMenuItem>
          ) : null}

          {onDelete ? (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                className="text-destructive focus:text-destructive"
                data-testid={`delete-message-${message.id}`}
                onClick={() => {
                  setIsDeleteDialogOpen(true);
                }}
              >
                <Trash2 className="h-4 w-4" />
                Delete message
              </DropdownMenuItem>
            </>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>

      {onDelete ? (
        <AlertDialog
          onOpenChange={setIsDeleteDialogOpen}
          open={isDeleteDialogOpen}
        >
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Delete message?</AlertDialogTitle>
              <AlertDialogDescription>
                This will permanently delete this message and cannot be undone.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel asChild>
                <Button type="button" variant="outline">
                  Cancel
                </Button>
              </AlertDialogCancel>
              <AlertDialogAction asChild>
                <Button
                  onClick={() => onDelete(message)}
                  type="button"
                  variant="destructive"
                >
                  Delete
                </Button>
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
    </>
  );
}

// ---------------------------------------------------------------------------
// MessageActionBar — reaction picker, reply button, and more-actions menu
// ---------------------------------------------------------------------------

function QuickReactionButton({
  customEmojiUrl,
  emoji,
  onSelect,
}: {
  customEmojiUrl?: string;
  emoji: string;
  onSelect: (emoji: string) => void;
}) {
  const displayName = emojiDisplayName(emoji);
  const mediaUrl = customEmojiUrl ? rewriteRelayUrl(customEmojiUrl) : null;

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          aria-label={`React with ${displayName}`}
          className="flex h-8 w-8 items-center justify-center rounded-full text-base leading-none text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
          onClick={() => onSelect(emoji)}
          title={displayName}
          type="button"
        >
          {mediaUrl ? (
            <img
              alt={emoji}
              className="h-5 w-5 object-contain"
              draggable={false}
              src={mediaUrl}
            />
          ) : (
            <span aria-hidden="true" className="translate-y-px">
              {emoji}
            </span>
          )}
        </button>
      </TooltipTrigger>
      <TooltipContent>{displayName}</TooltipContent>
    </Tooltip>
  );
}

function isCustomEmojiShortcode(emoji: string) {
  return emoji.startsWith(":") && emoji.endsWith(":");
}

export function MessageActionBar({
  channelId,
  message,
  onDelete,
  onEdit,
  onFollowThread,
  onMarkUnread,
  onReactionBadgeBurstRequest,
  onReactionSelect,
  onRemindLater,
  onReply,
  onUnfollowThread,
  reactionErrorMessage = null,
  reactions,
  isFollowingThread,
}: {
  /** Channel UUID — required for the "Copy link" action; when omitted the
   *  action is hidden (callers like the home inbox that lack the context). */
  channelId?: string | null;
  message: TimelineMessage;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onFollowThread?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onReactionBadgeBurstRequest?: (emoji: string) => void;
  onReactionSelect?: (emoji: string) => Promise<void>;
  onRemindLater?: (message: TimelineMessage) => void;
  onReply?: (message: TimelineMessage) => void;
  onUnfollowThread?: (message: TimelineMessage) => void;
  reactionErrorMessage?: string | null;
  reactions: TimelineReaction[];
  isFollowingThread?: boolean;
}) {
  const [isReactionPickerOpen, setIsReactionPickerOpen] = React.useState(false);
  const [isDropdownOpen, setIsDropdownOpen] = React.useState(false);
  const customEmoji = useCustomEmoji();
  const quickReactionEmojis = useQuickReactionEmojis(4, customEmoji);
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
    Boolean(onFollowThread) ||
    Boolean(onUnfollowThread) ||
    Boolean(onRemindLater) ||
    !message.pending;

  const wouldAddReaction = React.useCallback(
    (emoji: string) =>
      !reactions.some(
        (reaction) => reaction.emoji === emoji && reaction.reactedByCurrentUser,
      ),
    [reactions],
  );
  const handleReactionSelection = React.useCallback(
    (emoji: string, closePicker = false) => {
      if (!onReactionSelect) {
        return;
      }

      if (wouldAddReaction(emoji) && isPositiveEmojiParticle(emoji)) {
        onReactionBadgeBurstRequest?.(emoji);
      }

      void onReactionSelect(emoji)
        .then(() => {
          recordQuickReactionEmoji(emoji);
        })
        .catch(() => {})
        .finally(() => {
          if (closePicker) {
            setIsReactionPickerOpen(false);
          }
        });
    },
    [onReactionBadgeBurstRequest, onReactionSelect, wouldAddReaction],
  );

  if (!hasReplyAction && !hasReactionAction && !hasMoreMenuActions) {
    return null;
  }

  return (
    <div
      className={cn(
        "-m-1 p-1 transition-opacity duration-150 ease-out",
        "opacity-100 sm:pointer-events-none sm:opacity-0",
        "sm:group-hover/message:pointer-events-auto sm:group-hover/message:opacity-100",
        "sm:group-focus-within/message:pointer-events-auto sm:group-focus-within/message:opacity-100",
        isReactionPickerOpen || isDropdownOpen
          ? "sm:pointer-events-auto sm:opacity-100"
          : "",
      )}
      data-testid={`message-action-bar-${message.id}`}
    >
      <div className="overflow-hidden rounded-full border border-border/70 bg-background/95 shadow-xs backdrop-blur-sm supports-[backdrop-filter]:bg-background/85">
        <div className="flex items-center gap-0.5 p-1">
          {hasReactionAction && quickReactionItems.length > 0 ? (
            <>
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
              <div className="mx-0.5 hidden h-4 w-px bg-border/70 sm:block" />
            </>
          ) : null}

          {hasReactionAction ? (
            <Popover
              onOpenChange={setIsReactionPickerOpen}
              open={isReactionPickerOpen}
            >
              <Tooltip>
                <TooltipTrigger asChild>
                  <PopoverTrigger asChild>
                    <Button
                      aria-label="Open reactions"
                      className={ACTION_BUTTON_CLASS}
                      data-testid={`react-message-${message.id}`}
                      size="sm"
                      type="button"
                      variant={isReactionPickerOpen ? "secondary" : "ghost"}
                    >
                      <SmilePlus className={ACTION_ICON_CLASS} />
                    </Button>
                  </PopoverTrigger>
                </TooltipTrigger>
                <TooltipContent>React</TooltipContent>
              </Tooltip>
              <PopoverContent
                align="end"
                className="w-auto p-0 rounded-2xl overflow-hidden border-0 bg-transparent shadow-none"
                side="top"
                sideOffset={10}
              >
                {reactionErrorMessage ? (
                  <div className="px-3 pt-3 pb-0">
                    <p className="text-xs text-destructive">
                      {reactionErrorMessage}
                    </p>
                  </div>
                ) : null}
                <EmojiPicker
                  autoFocus
                  onSelect={(value) => {
                    // `value` is already a `native` glyph or a `:shortcode:` for
                    // custom emoji; the toggle mutation resolves the URL.
                    handleReactionSelection(value, true);
                  }}
                />
              </PopoverContent>
            </Popover>
          ) : null}

          {hasReplyAction ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  aria-label="Reply"
                  className={ACTION_BUTTON_CLASS}
                  data-testid={`reply-message-${message.id}`}
                  onClick={() => {
                    onReply?.(message);
                  }}
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

          {hasMoreMenuActions ? (
            <MoreActionsMenu
              channelId={channelId}
              message={message}
              onDelete={onDelete}
              onEdit={onEdit}
              onFollowThread={onFollowThread}
              onMarkUnread={onMarkUnread}
              onOpenChange={setIsDropdownOpen}
              onRemindLater={onRemindLater}
              onUnfollowThread={onUnfollowThread}
              open={isDropdownOpen}
              isFollowingThread={isFollowingThread}
            />
          ) : null}
        </div>
      </div>
    </div>
  );
}
