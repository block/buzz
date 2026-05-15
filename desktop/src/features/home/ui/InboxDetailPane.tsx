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
import type { TimelineMessage } from "@/features/messages/types";
import { MessageActionBar } from "@/features/messages/ui/MessageActionBar";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";

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
  onDelete: () => void;
  onSendReply: (input: {
    content: string;
    mediaTags?: string[][];
    mentionPubkeys: string[];
    parentEventId: string;
  }) => Promise<void>;
  onToggleDone: () => void;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
};

type InboxDisplayMessage = InboxContextMessage & {
  depth: number;
};

function toActionBarMessage(message: InboxDisplayMessage): TimelineMessage {
  return {
    id: message.id,
    author: message.authorLabel,
    avatarUrl: message.avatarUrl,
    body: message.content,
    createdAt: 0,
    depth: message.depth,
    reactions: message.reactions ?? [],
    time: message.fullTimestampLabel,
  };
}

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
  onDelete,
  onSendReply,
  onToggleDone,
  onToggleReaction,
}: InboxDetailPaneProps) {
  const detailPaneRef = React.useRef<HTMLElement | null>(null);
  const [replyTargetId, setReplyTargetId] = React.useState<string | null>(null);
  const [isFocusHighlightVisible, setIsFocusHighlightVisible] =
    React.useState(true);
  const selectedItemId = item?.id ?? null;

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
      {!canOpenChannel ? (
        <div className="px-6 pb-4 pt-14">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex min-w-0 items-center gap-3">
              <UserAvatar
                avatarUrl={item.avatarUrl}
                className="h-8 w-8 rounded-xl"
                displayName={item.senderLabel}
                size="md"
              />
              <div className="min-w-0">
                <div className="flex min-w-0 flex-wrap items-center gap-2">
                  <p className="truncate text-base font-semibold">
                    {item.senderLabel}
                  </p>
                  <span
                    className={cn(
                      "inline-flex items-center text-[10px] font-semibold uppercase tracking-[0.14em]",
                      item.isActionRequired
                        ? "text-amber-600 dark:text-amber-300"
                        : "text-primary",
                    )}
                  >
                    {item.categoryLabel}
                  </span>
                  {item.channelLabel ? (
                    <span className="inline-flex items-center text-[11px] font-medium text-muted-foreground">
                      #{item.channelLabel}
                    </span>
                  ) : null}
                </div>

                <div className="mt-1 flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
                  <span>{item.fullTimestampLabel}</span>
                  <span>Inbox only</span>
                </div>
              </div>
            </div>

            <div className="flex shrink-0 items-center gap-4">
              <TooltipProvider delayDuration={200}>
                <div className="flex items-center gap-4">
                  <div className="flex items-center gap-0.5">
                    <HeaderIconAction
                      label={isDone ? "Mark unread" : "Mark done"}
                      onClick={onToggleDone}
                      icon={
                        isDone ? (
                          <MailOpen className="h-4 w-4" />
                        ) : (
                          <CheckCheck className="h-4 w-4" />
                        )
                      }
                    />
                  </div>
                  {canDelete ? (
                    <HeaderMoreMenu
                      isDeletingMessage={isDeletingMessage}
                      onDelete={onDelete}
                    />
                  ) : null}
                </div>
              </TooltipProvider>
            </div>
          </div>
        </div>
      ) : null}

      <div className="relative min-h-0 flex-1 overflow-hidden">
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
                <div className="px-6 py-2">
                  <article
                    className={cn(
                      "group/message flex items-start gap-2.5 px-2 py-1 transition-colors duration-1000",
                      message.isSelected
                        ? cn(
                            isFocusHighlightVisible
                              ? "bg-primary/[0.07]"
                              : "bg-transparent",
                          )
                        : "hover:bg-muted/20",
                    )}
                    data-testid={
                      message.isSelected
                        ? "home-inbox-selected-message"
                        : "home-inbox-context-message"
                    }
                  >
                    <UserAvatar
                      avatarUrl={message.avatarUrl}
                      className="h-8 w-8 shrink-0 rounded-xl"
                      displayName={message.authorLabel}
                      size="md"
                    />
                    <div className="-mt-1 min-w-0 flex-1">
                      <div className="flex min-w-0 flex-wrap items-start gap-x-2 gap-y-0">
                        <p className="truncate text-sm font-semibold leading-none tracking-tight text-foreground">
                          {message.authorLabel}
                        </p>
                        {message.isSelected ? (
                          <span className="text-[10px] font-semibold uppercase leading-none tracking-[0.14em] text-muted-foreground/70">
                            Inbox item
                          </span>
                        ) : null}
                        <p className="ml-auto text-xs text-muted-foreground">
                          {message.fullTimestampLabel}
                        </p>
                        {canReply || onToggleReaction ? (
                          <div className="relative ml-1">
                            <div className="absolute right-0 top-1/2 -translate-y-1/2">
                              <MessageActionBar
                                activeReplyTargetId={replyTargetId}
                                message={toActionBarMessage(message)}
                                onReactionSelect={
                                  onToggleReaction
                                    ? (emoji) => {
                                        const actionBarMessage =
                                          toActionBarMessage(message);
                                        const remove =
                                          actionBarMessage.reactions?.some(
                                            (reaction) =>
                                              reaction.emoji === emoji &&
                                              reaction.reactedByCurrentUser,
                                          ) ?? false;
                                        return onToggleReaction(
                                          actionBarMessage,
                                          emoji,
                                          remove,
                                        );
                                      }
                                    : undefined
                                }
                                onReply={
                                  canReply
                                    ? () => handleSelectReplyTarget(message)
                                    : undefined
                                }
                                reactions={message.reactions ?? []}
                              />
                            </div>
                          </div>
                        ) : null}
                      </div>
                      <div className="mt-1">
                        <Markdown
                          className="max-w-full text-left text-sm text-foreground"
                          content={message.content}
                          mentionNames={message.mentionNames}
                          tight
                        />
                      </div>
                    </div>
                  </article>
                </div>
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

function HeaderIconAction({
  icon,
  label,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  onClick?: () => void;
}) {
  const button = (
    <Button
      aria-label={label}
      className="h-8 w-8 rounded-full p-0 text-muted-foreground"
      onClick={onClick}
      size="icon"
      type="button"
      variant="ghost"
    >
      {icon}
    </Button>
  );

  return (
    <Tooltip>
      <TooltipTrigger asChild>{button}</TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

function HeaderMoreMenu({
  isDeletingMessage,
  onDelete,
}: {
  isDeletingMessage: boolean;
  onDelete: () => void;
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
        <DropdownMenuItem
          className="text-destructive focus:text-destructive"
          disabled={isDeletingMessage}
          onClick={onDelete}
        >
          <Trash2 className="h-4 w-4" />
          Delete message
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
