import Picker from "@emoji-mart/react";
import data from "@emoji-mart/data";
import {
  Copy,
  CornerUpLeft,
  EllipsisVertical,
  MailOpen,
  Pencil,
  SmilePlus,
  Trash2,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import type {
  TimelineMessage,
  TimelineReaction,
} from "@/features/messages/types";
import { cn } from "@/shared/lib/cn";
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
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Spinner } from "@/shared/ui/spinner";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

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
  message,
  onDelete,
  onEdit,
  onMarkUnread,
  onOpenChange,
  open,
}: {
  message: TimelineMessage;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const [isDeleteDialogOpen, setIsDeleteDialogOpen] = React.useState(false);

  const hasCopyActions = !message.pending;

  return (
    <>
      <DropdownMenu open={open} onOpenChange={onOpenChange}>
        <Tooltip>
          <TooltipTrigger asChild>
            <DropdownMenuTrigger asChild>
              <Button
                aria-label="More actions"
                className="h-6 w-6 rounded-full p-0"
                data-testid={`more-actions-${message.id}`}
                size="sm"
                type="button"
                variant={open ? "secondary" : "ghost"}
              >
                <EllipsisVertical className="h-3 w-3" />
              </Button>
            </DropdownMenuTrigger>
          </TooltipTrigger>
          <TooltipContent>More actions</TooltipContent>
        </Tooltip>
        <DropdownMenuContent align="end" side="top" sideOffset={6}>
          {onEdit ? (
            <DropdownMenuItem
              data-testid={`edit-message-${message.id}`}
              onClick={() => {
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

export function MessageActionBar({
  activeReplyTargetId = null,
  message,
  onDelete,
  onEdit,
  onMarkUnread,
  onReactionSelect,
  onReply,
  reactionErrorMessage = null,
  reactions,
  reactionPending = false,
}: {
  activeReplyTargetId?: string | null;
  message: TimelineMessage;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onReactionSelect?: (emoji: string) => Promise<void>;
  onReply?: (message: TimelineMessage) => void;
  reactionErrorMessage?: string | null;
  reactions: TimelineReaction[];
  reactionPending?: boolean;
}) {
  const [isReactionPickerOpen, setIsReactionPickerOpen] = React.useState(false);
  const [isDropdownOpen, setIsDropdownOpen] = React.useState(false);
  const hasReplyAction = Boolean(onReply);
  const hasReactionAction = Boolean(onReactionSelect);

  const hasMoreMenuActions =
    Boolean(onEdit) ||
    Boolean(onDelete) ||
    Boolean(onMarkUnread) ||
    !message.pending;

  if (!hasReplyAction && !hasReactionAction && !hasMoreMenuActions) {
    return null;
  }

  const isReplyingToMessage = activeReplyTargetId === message.id;
  const selectedReactionCount = reactions.filter(
    (reaction) => reaction.reactedByCurrentUser,
  ).length;

  return (
    <div
      className={cn(
        "max-w-36 overflow-hidden rounded-full border border-border/70 bg-background/95 shadow-sm backdrop-blur supports-[backdrop-filter]:bg-background/85 transition-all duration-150 ease-out",
        "translate-y-0 opacity-100 sm:max-w-0 sm:border-0 sm:shadow-none sm:translate-y-1 sm:opacity-0",
        "sm:group-hover/message:max-w-36 sm:group-hover/message:border sm:group-hover/message:border-border/70 sm:group-hover/message:shadow-sm sm:group-hover/message:translate-y-0 sm:group-hover/message:opacity-100",
        "sm:group-focus-within/message:max-w-36 sm:group-focus-within/message:border sm:group-focus-within/message:border-border/70 sm:group-focus-within/message:shadow-sm sm:group-focus-within/message:translate-y-0 sm:group-focus-within/message:opacity-100",
        isReplyingToMessage || isReactionPickerOpen || isDropdownOpen
          ? "sm:max-w-36 sm:border sm:border-border/70 sm:shadow-sm sm:translate-y-0 sm:opacity-100"
          : "",
      )}
      data-testid={`message-action-bar-${message.id}`}
    >
      <div className="flex items-center gap-1 p-1">
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
                    className="h-6 w-6 rounded-full p-0"
                    data-testid={`react-message-${message.id}`}
                    disabled={reactionPending}
                    size="sm"
                    type="button"
                    variant={
                      isReactionPickerOpen || selectedReactionCount > 0
                        ? "secondary"
                        : "ghost"
                    }
                  >
                    {reactionPending ? (
                      <Spinner className="h-3 w-3" />
                    ) : (
                      <SmilePlus className="h-3 w-3" />
                    )}
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
              <Picker
                data={data}
                onEmojiSelect={(emoji: { native: string }) => {
                  if (!onReactionSelect) {
                    return;
                  }

                  void onReactionSelect(emoji.native).finally(() => {
                    setIsReactionPickerOpen(false);
                  });
                }}
                theme="auto"
                previewPosition="none"
                skinTonePosition="search"
                set="native"
                maxFrequentRows={2}
                perLine={8}
              />
            </PopoverContent>
          </Popover>
        ) : null}

        {hasReplyAction ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                aria-label={isReplyingToMessage ? "Cancel reply" : "Reply"}
                className="h-6 w-6 rounded-full p-0"
                data-testid={`reply-message-${message.id}`}
                onClick={() => {
                  onReply?.(message);
                }}
                size="sm"
                type="button"
                variant={isReplyingToMessage ? "secondary" : "ghost"}
              >
                <CornerUpLeft className="h-3 w-3" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>
              {isReplyingToMessage ? "Cancel reply" : "Reply"}
            </TooltipContent>
          </Tooltip>
        ) : null}

        {hasMoreMenuActions ? (
          <MoreActionsMenu
            message={message}
            onDelete={onDelete}
            onEdit={onEdit}
            onMarkUnread={onMarkUnread}
            onOpenChange={setIsDropdownOpen}
            open={isDropdownOpen}
          />
        ) : null}
      </div>
    </div>
  );
}
