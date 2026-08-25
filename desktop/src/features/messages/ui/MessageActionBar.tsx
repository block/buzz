import {
  BellOff,
  BellRing,
  Clock,
  Copy,
  CornerUpLeft,
  EllipsisVertical,
  Flag,
  Link2,
  MailCheck,
  MailOpen,
  Pencil,
  SmilePlus,
  Trash2,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { buildMessageLink } from "@/features/messages/lib/messageLink";
import { EmojiPicker } from "@/features/custom-emoji/ui/EmojiPicker";
import { getThreadReference } from "@/features/messages/lib/threading";
import { ReportMessageDialog } from "@/features/moderation/ui/ReportMessageDialog";
import { MessageModerationMenuItems } from "@/features/moderation/ui/MessageModerationMenuItems";
import type {
  TimelineMessage,
  TimelineReaction,
} from "@/features/messages/types";
import { recordQuickReactionEmoji } from "@/features/messages/ui/useQuickReactionEmojis";
import { cn } from "@/shared/lib/cn";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { KIND_HUDDLE_STARTED } from "@/shared/constants/kinds";
import { Button } from "@/shared/ui/button";
import { HashArrowIn } from "@/shared/ui/icons";
import { DeleteMessageConfirmDialog } from "./DeleteMessageConfirmDialog";
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

/** Copying a message link is offered from both the hover action bar and the
 *  More menu; both paths share this exact link-building + toast behavior. */
function copyMessageLink(channelId: string, message: TimelineMessage) {
  const { rootId } = getThreadReference(message.tags ?? []);
  const link = buildMessageLink({
    channelId,
    messageId: message.id,
    threadRootId: rootId,
  });
  copyTextToClipboard(link, "Link copied to clipboard");
}

/** Gate shared by every copy-link surface: pending sends have no delivered
 *  event to link to, huddle system rows aren't linkable, and callers without
 *  a channelId (e.g. inbox preview rows) can't build the link. */
function canCopyMessageLink(
  message: TimelineMessage,
  channelId: string | null | undefined,
): channelId is string {
  return (
    !message.pending &&
    message.kind !== KIND_HUDDLE_STARTED &&
    Boolean(channelId)
  );
}

function MoreActionsMenu({
  channelId,
  message,
  onDelete,
  onEdit,
  onFollowThread,
  onMarkUnread,
  onMarkRead,
  onOpenChange,
  onRemindLater,
  onSendToChannel,
  onUnfollowThread,
  open,
  isFollowingThread,
  isUnread,
}: {
  /** Channel UUID for the "Copy link" action. When null/undefined, the
   *  Copy link entry is hidden (e.g. inbox preview rows that don't have it). */
  channelId?: string | null;
  message: TimelineMessage;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onFollowThread?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onMarkRead?: (message: TimelineMessage) => void;
  onOpenChange: (open: boolean) => void;
  onRemindLater?: (message: TimelineMessage) => void;
  onSendToChannel?: (message: TimelineMessage) => Promise<void>;
  onUnfollowThread?: (message: TimelineMessage) => void;
  open: boolean;
  isFollowingThread?: boolean;
  isUnread?: boolean;
}) {
  const [isDeleteDialogOpen, setIsDeleteDialogOpen] = React.useState(false);
  const [isReportDialogOpen, setIsReportDialogOpen] = React.useState(false);
  // Set true the moment the user picks "Edit message". The
  // `onCloseAutoFocus` handler on `DropdownMenuContent` reads it to
  // suppress Radix's default focus-restoration (which would yank focus
  // back to the trigger and steal it from the composer's editor — the
  // composer schedules its own focus on RAF, but Radix's restoration
  // runs in a setTimeout that fires after our RAF and wins the race).
  // Reset to false inside the handler so Escape / non-Edit closes still
  // get default trigger-restoration (a11y intact for keyboard users).
  const editJustSelectedRef = React.useRef(false);

  const hasCopyActions =
    !message.pending && message.kind !== KIND_HUDDLE_STARTED;

  // A report needs a real, delivered event to target and a known author to
  // name in the NIP-56 `p` tag. Pending sends and system huddle rows have
  // neither, so the entry is hidden for them.
  const canReport =
    !message.pending &&
    message.kind !== KIND_HUDDLE_STARTED &&
    Boolean(message.pubkey);

  return (
    <>
      <DropdownMenu modal={false} open={open} onOpenChange={onOpenChange}>
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
              onSelect={() => {
                editJustSelectedRef.current = true;
                onEdit(message);
              }}
            >
              <Pencil className="h-4 w-4" />
              Edit message
            </DropdownMenuItem>
          ) : null}

          {onMarkRead || onMarkUnread ? (
            <DropdownMenuItem
              data-testid={`mark-read-toggle-${message.id}`}
              onClick={() => {
                if (isUnread) {
                  onMarkRead?.(message);
                } else {
                  onMarkUnread?.(message);
                }
              }}
            >
              {isUnread ? (
                <MailCheck className="h-4 w-4" />
              ) : (
                <MailOpen className="h-4 w-4" />
              )}
              {isUnread ? "Mark read" : "Mark unread"}
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
                copyTextToClipboard(
                  message.body,
                  "Message copied to clipboard",
                );
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

          {onSendToChannel ? (
            <DropdownMenuItem
              aria-label="Send to channel"
              data-testid={`send-to-channel-${message.id}`}
              onClick={() => {
                void onSendToChannel(message)
                  .then(() => toast.success("Sent to channel"))
                  .catch((error) => {
                    console.error(
                      "Failed to send thread message to channel",
                      error,
                    );
                    toast.error("Couldn't send to channel");
                  });
              }}
            >
              <HashArrowIn
                aria-hidden="true"
                className="h-4 w-4"
                data-testid="send-to-channel-icon"
              />
              Send to channel
            </DropdownMenuItem>
          ) : null}

          {canCopyMessageLink(message, channelId) ? (
            <DropdownMenuItem
              data-testid={`copy-message-link-${message.id}`}
              onClick={() => {
                copyMessageLink(channelId, message);
              }}
            >
              <Link2 className="h-4 w-4" />
              Copy link
            </DropdownMenuItem>
          ) : null}

          {canReport || onDelete ? <DropdownMenuSeparator /> : null}

          {canReport ? (
            <DropdownMenuItem
              data-testid={`report-message-${message.id}`}
              onClick={() => {
                setIsReportDialogOpen(true);
              }}
            >
              <Flag className="h-4 w-4" />
              Report message
            </DropdownMenuItem>
          ) : null}

          {onDelete ? (
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
          ) : null}

          {canReport ? (
            <MessageModerationMenuItems
              channelId={channelId}
              message={message}
            />
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>

      {onDelete ? (
        <DeleteMessageConfirmDialog
          onConfirm={() => onDelete(message)}
          onOpenChange={setIsDeleteDialogOpen}
          open={isDeleteDialogOpen}
        />
      ) : null}

      {canReport ? (
        <ReportMessageDialog
          open={isReportDialogOpen}
          onOpenChange={setIsReportDialogOpen}
          authorPubkey={message.pubkey ?? ""}
          eventId={message.id}
        />
      ) : null}
    </>
  );
}

export const MessageActionBar = React.memo(function MessageActionBar({
  channelId,
  message,
  onDelete,
  onEdit,
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
}: {
  /** Channel UUID — required for the "Copy link" action; when omitted the
   *  action is hidden (callers like the home inbox that lack the context). */
  channelId?: string | null;
  message: TimelineMessage;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onFollowThread?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onMarkRead?: (message: TimelineMessage) => void;
  onReactionBadgeBurstRequest?: (emoji: string) => void;
  onReactionSelect?: (emoji: string) => Promise<void>;
  onRemindLater?: (message: TimelineMessage) => void;
  onReply?: (message: TimelineMessage) => void;
  onSendToChannel?: (message: TimelineMessage) => Promise<void>;
  onUnfollowThread?: (message: TimelineMessage) => void;
  reactionErrorMessage?: string | null;
  reactions: TimelineReaction[];
  isFollowingThread?: boolean;
  /** Current read state of the clicked message, from the same predicate the
   *  unread badge uses. Drives the single mark-read/unread toggle label. */
  isUnread?: boolean;
}) {
  const [isReactionPickerOpen, setIsReactionPickerOpen] = React.useState(false);
  const [isDropdownOpen, setIsDropdownOpen] = React.useState(false);
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

          {canCopyMessageLink(message, channelId) ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  aria-label="Copy link"
                  className={ACTION_BUTTON_CLASS}
                  data-testid={`copy-link-message-${message.id}`}
                  onClick={() => {
                    copyMessageLink(channelId, message);
                  }}
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
            <MoreActionsMenu
              channelId={channelId}
              message={message}
              onDelete={onDelete}
              onEdit={onEdit}
              onFollowThread={onFollowThread}
              onMarkUnread={onMarkUnread}
              onMarkRead={onMarkRead}
              onOpenChange={setIsDropdownOpen}
              onRemindLater={onRemindLater}
              onSendToChannel={onSendToChannel}
              onUnfollowThread={onUnfollowThread}
              open={isDropdownOpen}
              isFollowingThread={isFollowingThread}
              isUnread={isUnread}
            />
          ) : null}
        </div>
      </div>
    </div>
  );
});

MessageActionBar.displayName = "MessageActionBar";
