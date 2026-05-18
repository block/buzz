import {
  CheckCheck,
  Mail,
  MailOpen,
  MoreHorizontal,
  Trash2,
} from "lucide-react";
import * as React from "react";

import type {
  InboxContextMessage,
  InboxItem,
  InboxReply,
} from "@/features/home/lib/inbox";
import {
  type InboxDisplayMessage,
  InboxMessageRow,
} from "@/features/home/ui/InboxMessageRow";
import type { TimelineMessage } from "@/features/messages/types";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/ui/tooltip";

type InboxDetailPaneProps = {
  canDelete: boolean;
  canOpenChannel: boolean;
  canReply: boolean;
  disabledReplyReason?: string | null;
  isDone: boolean;
  isDeletingMessage?: boolean;
  isSendingReply?: boolean;
  isThreadContextLoading?: boolean;
  item: InboxItem | null;
  messages?: InboxContextMessage[];
  replies?: InboxReply[];
  contextChannelName?: string | null;
  onDelete: () => void;
  onOpenContext?: (channelId: string, messageId: string) => void;
  onSendReply: (input: {
    content: string;
    mediaTags?: string[][];
    mentionPubkeys: string[];
    parentEventId: string;
  }) => Promise<void>;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  onToggleDone: () => void;
};

export function InboxDetailPane({
  canDelete,
  canOpenChannel,
  canReply,
  disabledReplyReason,
  isDone,
  isDeletingMessage = false,
  isSendingReply = false,
  isThreadContextLoading = false,
  item,
  messages = [],
  replies = [],
  contextChannelName = null,
  onDelete,
  onOpenContext,
  onSendReply,
  onToggleReaction,
  onToggleDone,
}: InboxDetailPaneProps) {
  const detailPaneRef = React.useRef<HTMLElement | null>(null);
  const [replyTargetId, setReplyTargetId] = React.useState<string | null>(null);
  const [isFocusHighlightVisible, setIsFocusHighlightVisible] =
    React.useState(true);
  const selectedItemId = item?.id ?? null;
  const selectedMessageScrollKey = React.useMemo(() => {
    if (!selectedItemId) {
      return null;
    }

    const selectedMessageIndex = messages.findIndex(
      (message) => message.isSelected,
    );
    return `${selectedItemId}:${selectedMessageIndex}:${messages.length}`;
  }, [messages, selectedItemId]);

  const focusComposer = React.useCallback(() => {
    window.requestAnimationFrame(() => {
      const textarea =
        detailPaneRef.current?.querySelector<HTMLTextAreaElement>(
          '[data-testid="message-input"]',
        );
      textarea?.focus();
    });
  }, []);

  React.useEffect(() => {
    void selectedItemId;
    setReplyTargetId(null);
  }, [selectedItemId]);

  React.useEffect(() => {
    void selectedItemId;
    setIsFocusHighlightVisible(true);
    const timeoutId = window.setTimeout(() => {
      setIsFocusHighlightVisible(false);
    }, 1_200);

    return () => {
      window.clearTimeout(timeoutId);
    };
  }, [selectedItemId]);

  React.useEffect(() => {
    if (!selectedMessageScrollKey) {
      return;
    }

    window.requestAnimationFrame(() => {
      detailPaneRef.current
        ?.querySelector<HTMLElement>(
          '[data-testid="home-inbox-selected-message"]',
        )
        ?.scrollIntoView({ block: "center" });
    });
  }, [selectedMessageScrollKey]);

  if (!item) {
    return (
      <section
        className="flex min-h-0 min-w-0 items-center justify-center bg-background/60 px-6 py-10 pt-20 text-center"
        data-testid="home-inbox-detail-empty"
      >
        <div className="max-w-sm">
          <div className="mx-auto flex h-14 w-14 items-center justify-center rounded-full bg-muted text-muted-foreground">
            <Mail className="h-6 w-6" />
          </div>
          <p className="mt-4 text-base font-semibold">Select a message</p>
          <p className="mt-1 text-sm text-muted-foreground">
            Pick an inbox item to see the full message and react to it.
          </p>
        </div>
      </section>
    );
  }

  const selectedMessage = messages.find((message) => message.isSelected);
  const pendingReplyMessages: InboxDisplayMessage[] = replies.map((reply) => ({
    ...reply,
    depth: reply.depth ?? (selectedMessage?.depth ?? 0) + 1,
    isSelected: false,
    mentionNames: [],
  }));
  const displayMessages: InboxDisplayMessage[] =
    messages.length > 0
      ? [...messages, ...pendingReplyMessages]
      : [
          {
            authorLabel: item.senderLabel,
            avatarUrl: item.avatarUrl,
            content: item.preview,
            depth: 0,
            fullTimestampLabel: item.fullTimestampLabel,
            id: item.id,
            isSelected: true,
            mentionNames: item.mentionNames,
          },
          ...pendingReplyMessages,
        ];
  const replyTarget =
    displayMessages.find((message) => message.id === replyTargetId) ?? null;
  const composerParentEventId = replyTarget?.id ?? item.id;
  const composerReplyTarget =
    replyTarget && replyTarget.id !== item.id
      ? {
          author: replyTarget.authorLabel,
          body: replyTarget.content,
          id: replyTarget.id,
        }
      : null;
  const channelContextName = contextChannelName ?? item.channelLabel;
  const contextLabel = channelContextName
    ? `#${channelContextName}`
    : item.categoryLabel;
  const contextChannelId = item.item.channelId;

  const handleSelectReplyTarget = (message: InboxDisplayMessage) => {
    setReplyTargetId((currentReplyTargetId) =>
      currentReplyTargetId === message.id ? null : message.id,
    );
    focusComposer();
  };

  return (
    <section
      className="flex min-h-0 min-w-0 flex-col overflow-hidden bg-background/60"
      data-testid="home-inbox-detail"
      ref={detailPaneRef}
    >
      <div className="relative min-h-0 flex-1 overflow-hidden">
        <div className="absolute inset-x-0 top-0 z-40 flex min-h-[44px] items-center justify-between gap-3 bg-background/70 py-[6px] pl-6 pr-3 backdrop-blur-xl supports-[backdrop-filter]:bg-background/55">
          <div className="min-w-0">
            {canOpenChannel && contextChannelId && onOpenContext ? (
              <button
                className="truncate text-left text-sm font-semibold leading-5 tracking-tight text-foreground hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
                onClick={() => onOpenContext(contextChannelId, item.id)}
                title={item.fullTimestampLabel}
                type="button"
              >
                {contextLabel}
              </button>
            ) : (
              <h2
                className="truncate text-sm font-semibold leading-5 tracking-tight text-foreground"
                title={item.fullTimestampLabel}
              >
                {contextLabel}
              </h2>
            )}
          </div>

          <TooltipProvider delayDuration={200}>
            <div className="flex shrink-0 items-center gap-1">
              <HeaderMoreMenu
                canDelete={canDelete}
                isDeletingMessage={isDeletingMessage}
                isDone={isDone}
                onDelete={onDelete}
                onToggleDone={onToggleDone}
              />
            </div>
          </TooltipProvider>
        </div>

        <div className="absolute inset-0 overflow-y-auto overscroll-contain pb-32 pt-14">
          <div>
            {isThreadContextLoading ? (
              <div className="px-6 pb-3 text-[11px] text-muted-foreground">
                Loading context...
              </div>
            ) : null}
            {displayMessages.map((message, index) => (
              <React.Fragment key={message.id}>
                {index === 1 ? (
                  <div className="mx-6 my-3 border-t border-border/60" />
                ) : null}
                <InboxMessageRow
                  activeReplyTargetId={replyTargetId}
                  canReply={canReply}
                  isFocusHighlightVisible={isFocusHighlightVisible}
                  message={message}
                  onSelectReplyTarget={handleSelectReplyTarget}
                  onToggleReaction={onToggleReaction}
                />
              </React.Fragment>
            ))}
          </div>
        </div>

        <div className="pointer-events-none absolute inset-x-0 bottom-0 z-10">
          <div className="pointer-events-auto">
            <MessageComposer
              channelId={item.item.channelId}
              channelName={item.channelLabel ?? "channel"}
              containerClassName="px-6 pb-4 sm:px-6 [&>div]:max-w-none"
              disabled={!canReply}
              draftKey={`inbox-reply:${item.id}`}
              isSending={isSendingReply}
              onCancelReply={
                composerReplyTarget ? () => setReplyTargetId(null) : undefined
              }
              onSend={(content, mentionPubkeys, mediaTags) =>
                onSendReply({
                  content,
                  mediaTags,
                  mentionPubkeys,
                  parentEventId: composerParentEventId,
                })
              }
              placeholder={
                canReply
                  ? `Send reply to ${item.channelLabel ? `#${item.channelLabel} thread` : "channel thread"}`
                  : (disabledReplyReason ??
                    "Replies are not available for this item.")
              }
              replyTarget={composerReplyTarget}
            />
          </div>
        </div>
      </div>
    </section>
  );
}

function HeaderMoreMenu({
  canDelete,
  isDeletingMessage,
  isDone,
  onDelete,
  onToggleDone,
}: {
  canDelete: boolean;
  isDeletingMessage: boolean;
  isDone: boolean;
  onDelete: () => void;
  onToggleDone: () => void;
}) {
  const trigger = (
    <Button
      aria-label="More actions"
      className="h-8 w-8 rounded-full p-0 text-muted-foreground"
      size="icon"
      type="button"
      variant="ghost"
    >
      <MoreHorizontal className="h-4 w-4" />
    </Button>
  );

  return (
    <DropdownMenu modal={false}>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>{trigger}</DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent>More actions</TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onClick={onToggleDone}>
          {isDone ? (
            <MailOpen className="h-4 w-4" />
          ) : (
            <CheckCheck className="h-4 w-4" />
          )}
          {isDone ? "Unmark as read" : "Mark as read"}
        </DropdownMenuItem>
        {canDelete ? <DropdownMenuSeparator /> : null}
        {canDelete ? (
          <DropdownMenuItem
            className="text-destructive focus:text-destructive"
            disabled={isDeletingMessage}
            onClick={onDelete}
          >
            <Trash2 className="h-4 w-4" />
            Delete message
          </DropdownMenuItem>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
