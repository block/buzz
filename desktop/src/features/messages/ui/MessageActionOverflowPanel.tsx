import {
  BellOff,
  BellRing,
  Clock,
  Copy,
  Flag,
  Link2,
  MailCheck,
  MailOpen,
  Pencil,
  Trash2,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { KIND_HUDDLE_STARTED } from "@/shared/constants/kinds";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { HashArrowIn } from "@/shared/ui/icons";
import { MessageModerationInlineItems } from "@/features/moderation/ui/MessageModerationMenuItems";
import { ReportMessageDialog } from "@/features/moderation/ui/ReportMessageDialog";
import type { TimelineMessage } from "@/features/messages/types";
import { DeleteMessageConfirmDialog } from "@/features/messages/ui/DeleteMessageConfirmDialog";
import {
  canCopyMessageLink,
  copyMessageLink,
} from "@/features/messages/ui/MessageActionToolbarHelpers";

export type MessageActionOverflowPanelProps = {
  channelId?: string | null;
  message: TimelineMessage;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onFollowThread?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onMarkRead?: (message: TimelineMessage) => void;
  onClose: () => void;
  onRemindLater?: (message: TimelineMessage) => void;
  onSendToChannel?: (message: TimelineMessage) => Promise<void>;
  onUnfollowThread?: (message: TimelineMessage) => void;
  isFollowingThread?: boolean;
  isUnread?: boolean;
};

export function MessageActionOverflowPanel({
  channelId,
  message,
  onDelete,
  onEdit,
  onFollowThread,
  onMarkUnread,
  onMarkRead,
  onClose,
  onRemindLater,
  onSendToChannel,
  onUnfollowThread,
  isFollowingThread,
  isUnread,
}: MessageActionOverflowPanelProps) {
  const [isDeleteDialogOpen, setIsDeleteDialogOpen] = React.useState(false);
  const [isReportDialogOpen, setIsReportDialogOpen] = React.useState(false);
  const panelRef = React.useRef<HTMLDivElement>(null);
  const hasCopyActions =
    !message.pending && message.kind !== KIND_HUDDLE_STARTED;
  const canReport =
    !message.pending &&
    message.kind !== KIND_HUDDLE_STARTED &&
    Boolean(message.pubkey);
  const itemClassName =
    "flex min-h-9 w-full select-none items-center gap-2 rounded-lg py-2 pl-2 pr-4 text-left text-sm outline-hidden transition-colors hover:bg-muted/50 focus-visible:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring [&>svg]:size-4 [&>svg]:shrink-0";

  React.useEffect(() => {
    panelRef.current
      ?.querySelector<HTMLElement>('[role="menuitem"]:not([disabled])')
      ?.focus();
  }, []);

  const handleMenuKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    const items = Array.from(
      event.currentTarget.querySelectorAll<HTMLElement>(
        '[role="menuitem"]:not([disabled])',
      ),
    );
    if (items.length === 0) return;
    event.preventDefault();
    const currentIndex = items.indexOf(document.activeElement as HTMLElement);
    const nextIndex =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? items.length - 1
          : event.key === "ArrowDown"
            ? (currentIndex + 1) % items.length
            : (currentIndex - 1 + items.length) % items.length;
    items[nextIndex]?.focus();
  };

  return (
    <>
      <div
        className="max-h-[min(420px,calc(100vh-3rem))] min-w-60 overflow-y-auto p-1"
        data-testid={`more-actions-panel-${message.id}`}
        onKeyDown={handleMenuKeyDown}
        ref={panelRef}
        role="menu"
      >
        {onEdit ? (
          <button
            className={itemClassName}
            data-testid={`edit-message-${message.id}`}
            onClick={() => {
              onClose();
              onEdit(message);
            }}
            role="menuitem"
            type="button"
          >
            <Pencil />
            Edit message
          </button>
        ) : null}

        {onMarkRead || onMarkUnread ? (
          <button
            className={itemClassName}
            data-testid={`mark-read-toggle-${message.id}`}
            onClick={() => {
              if (isUnread) onMarkRead?.(message);
              else onMarkUnread?.(message);
              onClose();
            }}
            role="menuitem"
            type="button"
          >
            {isUnread ? <MailCheck /> : <MailOpen />}
            {isUnread ? "Mark read" : "Mark unread"}
          </button>
        ) : null}

        {onFollowThread || onUnfollowThread ? (
          <button
            className={itemClassName}
            onClick={() => {
              if (isFollowingThread) onUnfollowThread?.(message);
              else onFollowThread?.(message);
              onClose();
            }}
            role="menuitem"
            type="button"
          >
            {isFollowingThread ? <BellOff /> : <BellRing />}
            {isFollowingThread ? "Unfollow thread" : "Follow thread"}
          </button>
        ) : null}

        {hasCopyActions ? (
          <button
            className={itemClassName}
            onClick={() => {
              copyTextToClipboard(message.body, "Message copied to clipboard");
              onClose();
            }}
            role="menuitem"
            type="button"
          >
            <Copy />
            Copy message
          </button>
        ) : null}

        {onRemindLater ? (
          <button
            className={itemClassName}
            onClick={() => {
              onRemindLater(message);
              onClose();
            }}
            role="menuitem"
            type="button"
          >
            <Clock />
            Remind me later
          </button>
        ) : null}

        {onSendToChannel ? (
          <button
            aria-label="Send to channel"
            className={itemClassName}
            data-testid={`send-to-channel-${message.id}`}
            onClick={() => {
              onClose();
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
            role="menuitem"
            type="button"
          >
            <HashArrowIn
              aria-hidden="true"
              data-testid="send-to-channel-icon"
            />
            Send to channel
          </button>
        ) : null}

        {canCopyMessageLink(message, channelId) ? (
          <button
            className={itemClassName}
            data-testid={`copy-message-link-${message.id}`}
            onClick={() => {
              copyMessageLink(channelId, message);
              onClose();
            }}
            role="menuitem"
            type="button"
          >
            <Link2 />
            Copy link
          </button>
        ) : null}

        {canReport || onDelete ? (
          <hr className="-mx-1 my-1 border-0 border-t border-muted" />
        ) : null}
        {canReport ? (
          <button
            className={itemClassName}
            data-testid={`report-message-${message.id}`}
            onClick={() => setIsReportDialogOpen(true)}
            role="menuitem"
            type="button"
          >
            <Flag />
            Report message
          </button>
        ) : null}
        {onDelete ? (
          <button
            className={`${itemClassName} text-destructive`}
            data-testid={`delete-message-${message.id}`}
            onClick={() => setIsDeleteDialogOpen(true)}
            role="menuitem"
            type="button"
          >
            <Trash2 />
            Delete message
          </button>
        ) : null}
        <MessageModerationInlineItems
          channelId={channelId}
          message={message}
          onAction={onClose}
        />
      </div>

      {onDelete ? (
        <DeleteMessageConfirmDialog
          onConfirm={() => onDelete(message)}
          onOpenChange={(nextOpen) => {
            setIsDeleteDialogOpen(nextOpen);
            if (!nextOpen) onClose();
          }}
          open={isDeleteDialogOpen}
        />
      ) : null}
      {canReport ? (
        <ReportMessageDialog
          authorPubkey={message.pubkey ?? ""}
          eventId={message.id}
          onOpenChange={(nextOpen) => {
            setIsReportDialogOpen(nextOpen);
            if (!nextOpen) onClose();
          }}
          open={isReportDialogOpen}
        />
      ) : null}
    </>
  );
}
