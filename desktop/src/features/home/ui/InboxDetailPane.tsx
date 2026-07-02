import { ArrowLeft, Hash, Mail, MoreHorizontal, Trash2 } from "lucide-react";
import * as React from "react";

import type {
  InboxContextMessage,
  InboxItem,
  InboxReply,
} from "@/features/home/lib/inbox";
import { ChannelMembersBar } from "@/features/channels/ui/ChannelMembersBar";
import { formatInboxTypeLabel } from "@/features/home/lib/inbox";
import {
  type InboxDisplayMessage,
  InboxMessageRow,
} from "@/features/home/ui/InboxMessageRow";
import type { TimelineMessage } from "@/features/messages/types";
import { formatTime } from "@/features/messages/lib/dateFormatters";
import { hasSameMessageAuthor } from "@/features/messages/lib/messageGrouping";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { UpdateIndicator } from "@/features/settings/UpdateIndicator";
import type { Channel } from "@/shared/api/types";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
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

const MembersSidebar = React.lazy(async () => {
  const module = await import("@/features/channels/ui/MembersSidebar");
  return { default: module.MembersSidebar };
});

type InboxDetailPaneProps = {
  agentPubkeys?: ReadonlySet<string>;
  canDelete: boolean;
  canOpenChannel: boolean;
  canReply: boolean;
  disabledReplyReason?: string | null;
  isDeletingMessage?: boolean;
  isSendingReply?: boolean;
  isSinglePanelView?: boolean;
  isThreadContextLoading?: boolean;
  item: InboxItem | null;
  messages?: InboxContextMessage[];
  replies?: InboxReply[];
  channel: Channel | null;
  contextChannelName?: string | null;
  currentPubkey?: string;
  onBack?: () => void;
  onDelete: () => void;
  onOpenChannel: (channelId: string) => void;
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
};

export function InboxDetailPane({
  agentPubkeys,
  canDelete,
  canOpenChannel,
  canReply,
  disabledReplyReason,
  isDeletingMessage = false,
  isSendingReply = false,
  isSinglePanelView = false,
  isThreadContextLoading = false,
  item,
  messages = [],
  replies = [],
  channel,
  contextChannelName = null,
  currentPubkey,
  onBack,
  onDelete,
  onOpenChannel,
  onSendReply,
  onToggleReaction,
}: InboxDetailPaneProps) {
  const detailPaneRef = React.useRef<HTMLElement | null>(null);
  const [replyTargetId, setReplyTargetId] = React.useState<string | null>(null);
  const [isFocusHighlightVisible, setIsFocusHighlightVisible] =
    React.useState(true);
  const [isMembersSidebarOpen, setIsMembersSidebarOpen] = React.useState(false);
  const selectedItemId = item?.id ?? null;
  const selectedChannelId = item?.item.channelId ?? null;
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
    void selectedChannelId;
    setIsMembersSidebarOpen(false);
  }, [selectedChannelId]);

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
            authorPubkey: item.item.pubkey,
            avatarUrl: item.avatarUrl,
            content: item.preview,
            depth: 0,
            fullTimestampLabel: item.fullTimestampLabel,
            id: item.id,
            isSelected: true,
            mentionNames: item.mentionNames,
            timeLabel: formatTime(item.item.createdAt),
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
  const composerChannelType =
    item.item.channelType === "dm" ||
    item.item.channelType === "stream" ||
    item.item.channelType === "forum"
      ? item.item.channelType
      : null;
  const contextLabel = channelContextName ?? formatInboxTypeLabel(item);
  const hasChannelContext = Boolean(channelContextName);
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
      <div className="relative flex min-h-0 flex-1 flex-col overflow-hidden">
        <TopChromeInsetHeader flush>
          <div className="px-5 py-2">
            <div className="flex min-h-9 min-w-0 items-center justify-between gap-3">
              <div
                className={cn(
                  "flex min-w-0 items-center",
                  isSinglePanelView ? "gap-[4px]" : "gap-1",
                )}
              >
                {onBack ? (
                  <Button
                    aria-label="Back to inbox list"
                    className="rounded-full text-muted-foreground hover:bg-muted/60 hover:text-foreground"
                    onClick={onBack}
                    size="icon"
                    type="button"
                    variant="ghost"
                  >
                    <ArrowLeft />
                  </Button>
                ) : null}
                <div className="min-w-0">
                  {canOpenChannel && contextChannelId ? (
                    <button
                      className="flex min-w-0 items-center gap-[4px] text-left text-sm font-semibold leading-5 tracking-tight text-foreground hover:underline focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
                      onClick={() => onOpenChannel(contextChannelId)}
                      title={item.fullTimestampLabel}
                      type="button"
                    >
                      {hasChannelContext ? (
                        <Hash className="h-4 w-4 shrink-0" color="gray" />
                      ) : null}
                      <span className="min-w-0 translate-y-px truncate">
                        {contextLabel}
                      </span>
                    </button>
                  ) : (
                    <h2
                      className="flex min-w-0 items-center gap-[4px] text-sm font-semibold leading-5 tracking-tight text-foreground"
                      title={item.fullTimestampLabel}
                    >
                      {hasChannelContext ? (
                        <Hash className="h-4 w-4 shrink-0" color="gray" />
                      ) : null}
                      <span className="min-w-0 translate-y-px truncate">
                        {contextLabel}
                      </span>
                    </h2>
                  )}
                </div>
              </div>

              <TooltipProvider delayDuration={200}>
                <div className="flex shrink-0 items-center gap-1">
                  <UpdateIndicator />
                  {channel ? (
                    <ChannelMembersBar
                      channel={channel}
                      currentPubkey={currentPubkey}
                      onManageChannel={() => {
                        if (contextChannelId) {
                          onOpenChannel(contextChannelId);
                        }
                      }}
                      onToggleMembers={() =>
                        setIsMembersSidebarOpen((open) => !open)
                      }
                    />
                  ) : null}
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
        </TopChromeInsetHeader>

        <div
          aria-busy={isThreadContextLoading}
          className="min-h-0 flex-1 overflow-y-auto overscroll-contain pb-32"
        >
          <div>
            {displayMessages.map((message, index) => {
              const isAfterSeparator = index === 1;
              const isContinuation =
                !isAfterSeparator &&
                hasSameMessageAuthor(
                  { pubkey: displayMessages[index - 1]?.authorPubkey },
                  { pubkey: message.authorPubkey },
                );

              return (
                <React.Fragment key={message.id}>
                  {isAfterSeparator ? (
                    <div className="mx-6 my-3 border-t border-border/60" />
                  ) : null}
                  <InboxMessageRow
                    agentPubkeys={agentPubkeys}
                    canReply={canReply}
                    channelId={item.item.channelId}
                    isContinuation={isContinuation}
                    isFocusHighlightVisible={isFocusHighlightVisible}
                    message={message}
                    onSelectReplyTarget={handleSelectReplyTarget}
                    onToggleReaction={onToggleReaction}
                  />
                </React.Fragment>
              );
            })}
          </div>
        </div>

        <div className="pointer-events-none absolute inset-x-0 bottom-0 z-10">
          <div className="pointer-events-auto">
            <MessageComposer
              channelId={item.item.channelId}
              channelName={item.channelLabel ?? "channel"}
              channelType={composerChannelType}
              containerClassName="px-4 pb-4 sm:px-4"
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

      {channel ? (
        <React.Suspense fallback={null}>
          <MembersSidebar
            channel={channel}
            currentPubkey={currentPubkey}
            onOpenChange={setIsMembersSidebarOpen}
            open={isMembersSidebarOpen}
          />
        </React.Suspense>
      ) : null}
    </section>
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
      className="rounded-full text-muted-foreground"
      size="icon"
      type="button"
      variant="ghost"
    >
      <MoreHorizontal />
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
