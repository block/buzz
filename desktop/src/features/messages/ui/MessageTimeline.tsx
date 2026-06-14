import * as React from "react";
import { ArrowDown, Hash } from "lucide-react";
import type { VirtuosoHandle } from "react-virtuoso";

import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";
import { isSameDay } from "@/features/messages/lib/dateFormatters";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { cn } from "@/shared/lib/cn";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";
import { TooltipProvider } from "@/shared/ui/tooltip";
import { TimelineSkeleton } from "./TimelineSkeleton";
import { VirtualizedTimelineMessageList } from "./VirtualizedTimelineMessageList";

type MessageTimelineProps = {
  agentPubkeys?: ReadonlySet<string>;
  channelId?: string | null;
  channelIntro?: ChannelIntro | null;
  channelName?: string;
  channelType?: ChannelType | null;
  messages: TimelineMessage[];
  directMessageIntro?: {
    avatarUrl: string | null;
    displayName: string;
  } | null;
  isLoading?: boolean;
  emptyTitle?: string;
  emptyDescription?: string;
  currentPubkey?: string;
  fetchOlder?: () => Promise<void>;
  hasOlderMessages?: boolean;
  /** Optional external ref to the scroll container — used by the parent to
   *  observe scroll position or adjust padding dynamically. */
  scrollContainerRef?: React.RefObject<HTMLDivElement | null>;
  /** True when the timeline has the composer overlay below it. */
  hasComposerOverlay?: boolean;
  isFetchingOlder?: boolean;
  messageFooters?: Record<string, React.ReactNode>;
  /** Map from lowercase pubkey → persona display name for bot members. */
  personaLookup?: Map<string, string>;
  profiles?: UserProfileLookup;
  followThreadById?: (rootId: string) => void;
  isFollowingThreadById?: (rootId: string) => boolean;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onReply?: (message: TimelineMessage) => void;
  isSendingVideoReviewComment?: boolean;
  onSendVideoReviewComment?: (
    message: TimelineMessage,
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    parentEventId?: string,
  ) => Promise<void>;
  unfollowThreadById?: (rootId: string) => void;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  /** The message ID of the currently active find-in-channel match. */
  searchActiveMessageId?: string | null;
  /** Set of message IDs that match the current find-in-channel query. */
  searchMatchingMessageIds?: Set<string>;
  /** The current find-in-channel query string. */
  searchQuery?: string;
  targetMessageId?: string | null;
  onTargetReached?: (messageId: string) => void;
};

type ChannelIntroAction = {
  description?: string;
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
  testId?: string;
};

type ChannelIntro = {
  actions?: ChannelIntroAction[];
  channelKindLabel: string;
  channelName: string;
  description?: string | null;
  icon?: React.ReactNode;
};

function mergeRefs<T>(...refs: Array<React.Ref<T> | undefined>) {
  return (value: T | null) => {
    for (const ref of refs) {
      if (!ref) continue;
      if (typeof ref === "function") {
        ref(value);
      } else {
        ref.current = value;
      }
    }
  };
}

function getTimelineItemIndex(messages: TimelineMessage[], messageId: string) {
  let itemIndex = 0;
  for (let index = 0; index < messages.length; index += 1) {
    const message = messages[index];
    const previous = index > 0 ? messages[index - 1] : null;
    if (!previous || !isSameDay(previous.createdAt, message.createdAt)) {
      itemIndex += 1;
    }
    if (message.id === messageId) {
      return itemIndex;
    }
    itemIndex += 1;
  }
  return -1;
}

export const MessageTimeline = React.memo(function MessageTimeline({
  agentPubkeys,
  channelId,
  channelIntro = null,
  directMessageIntro = null,
  messages,
  isLoading = false,
  emptyTitle = "No messages yet",
  emptyDescription = "Send the first message to start the thread.",
  currentPubkey,
  fetchOlder,
  hasComposerOverlay = true,
  hasOlderMessages = true,
  isFetchingOlder = false,
  followThreadById,
  isFollowingThreadById,
  messageFooters,
  personaLookup,
  profiles,
  onDelete,
  onEdit,
  onMarkUnread,
  onReply,
  channelName,
  channelType,
  isSendingVideoReviewComment = false,
  onSendVideoReviewComment,
  onToggleReaction,
  unfollowThreadById,
  scrollContainerRef: externalScrollRef,
  searchActiveMessageId = null,
  searchMatchingMessageIds,
  searchQuery,
  targetMessageId = null,
  onTargetReached,
}: MessageTimelineProps) {
  const internalScrollRef = React.useRef<HTMLDivElement>(null);
  const virtuosoRef = React.useRef<VirtuosoHandle>(null);
  const scrollContainerRef = externalScrollRef ?? internalScrollRef;
  const fetchOlderInFlightRef = React.useRef(false);
  const hasInitializedScrollRef = React.useRef(false);
  const shouldStickToBottomRef = React.useRef(true);
  const handledTargetMessageIdRef = React.useRef<string | null>(null);
  const previousLastMessageKeyRef = React.useRef<string | undefined>(undefined);
  const previousMessageCountRef = React.useRef(0);
  const previousChannelIdRef = React.useRef(channelId);
  const [highlightedMessageId, setHighlightedMessageId] = React.useState<
    string | null
  >(null);
  const [isAtBottom, setIsAtBottom] = React.useState(true);
  const [newMessageCount, setNewMessageCount] = React.useState(0);
  const scrollRestorationId = targetMessageId
    ? `message-timeline:${channelId ?? "none"}:target:${targetMessageId}`
    : `message-timeline:${channelId ?? "none"}`;

  React.useLayoutEffect(() => {
    if (previousChannelIdRef.current === channelId) {
      return;
    }

    previousChannelIdRef.current = channelId;
    hasInitializedScrollRef.current = false;
    shouldStickToBottomRef.current = true;
    handledTargetMessageIdRef.current = null;
    previousLastMessageKeyRef.current = undefined;
    previousMessageCountRef.current = 0;
    setHighlightedMessageId(null);
    setIsAtBottom(true);
    setNewMessageCount(0);
  });

  const setScrollerRef = React.useMemo(
    () => mergeRefs<HTMLDivElement>(scrollContainerRef),
    [scrollContainerRef],
  );

  const showDirectMessageIntro = !isLoading && directMessageIntro !== null;
  const showChannelIntro =
    !isLoading && channelIntro !== null && directMessageIntro === null;
  const showIntro = showDirectMessageIntro || showChannelIntro;
  const showGenericEmpty =
    !isLoading &&
    messages.length === 0 &&
    directMessageIntro === null &&
    channelIntro === null;
  const showMessageList = !isLoading && messages.length > 0;
  const latestMessage =
    messages.length > 0 ? messages[messages.length - 1] : undefined;
  const latestMessageKey = latestMessage
    ? (latestMessage.renderKey ?? latestMessage.id)
    : undefined;

  const scrollToBottom = React.useCallback(
    (behavior: ScrollBehavior) => {
      if (messages.length === 0) return;
      shouldStickToBottomRef.current = true;
      setIsAtBottom(true);
      setNewMessageCount(0);
      virtuosoRef.current?.scrollToIndex({
        align: "end",
        behavior: behavior === "smooth" ? "smooth" : "auto",
        index: "LAST",
      });
    },
    [messages.length],
  );

  const handleAtBottomStateChange = React.useCallback((atBottom: boolean) => {
    shouldStickToBottomRef.current = atBottom;
    setIsAtBottom(atBottom);
    if (atBottom) {
      setNewMessageCount(0);
    }
  }, []);

  const handleStartReached = React.useCallback(() => {
    if (
      !fetchOlder ||
      !hasOlderMessages ||
      isLoading ||
      isFetchingOlder ||
      fetchOlderInFlightRef.current
    ) {
      return;
    }

    fetchOlderInFlightRef.current = true;
    void fetchOlder().finally(() => {
      fetchOlderInFlightRef.current = false;
    });
  }, [fetchOlder, hasOlderMessages, isFetchingOlder, isLoading]);

  React.useLayoutEffect(() => {
    if (isLoading || !showMessageList) {
      return;
    }

    if (!hasInitializedScrollRef.current) {
      hasInitializedScrollRef.current = true;
      previousLastMessageKeyRef.current = latestMessageKey;
      previousMessageCountRef.current = messages.length;

      if (!targetMessageId) {
        scrollToBottom("auto");
      }
      return;
    }

    const previousLastMessageKey = previousLastMessageKeyRef.current;
    const previousMessageCount = previousMessageCountRef.current;
    const hasNewLatestMessage =
      latestMessage !== undefined &&
      latestMessageKey !== previousLastMessageKey;

    if (hasNewLatestMessage) {
      if (
        !targetMessageId &&
        (shouldStickToBottomRef.current || latestMessage.accent)
      ) {
        scrollToBottom(latestMessage.accent ? "smooth" : "auto");
      } else {
        setNewMessageCount((current) => {
          const addedMessages = Math.max(
            1,
            messages.length - previousMessageCount,
          );
          return current + addedMessages;
        });
      }
    }

    previousLastMessageKeyRef.current = latestMessageKey;
    previousMessageCountRef.current = messages.length;
  }, [
    isLoading,
    latestMessage,
    latestMessageKey,
    messages.length,
    scrollToBottom,
    showMessageList,
    targetMessageId,
  ]);

  React.useEffect(() => {
    if (!searchActiveMessageId) return;
    const index = getTimelineItemIndex(messages, searchActiveMessageId);
    if (index < 0) return;
    virtuosoRef.current?.scrollToIndex({
      align: "center",
      behavior: "smooth",
      index,
    });
  }, [messages, searchActiveMessageId]);

  React.useEffect(() => {
    if (!targetMessageId) {
      handledTargetMessageIdRef.current = null;
      setHighlightedMessageId(null);
      return;
    }

    if (handledTargetMessageIdRef.current === targetMessageId || isLoading) {
      return;
    }

    const index = getTimelineItemIndex(messages, targetMessageId);
    if (index < 0) return;

    handledTargetMessageIdRef.current = targetMessageId;
    shouldStickToBottomRef.current = false;
    setHighlightedMessageId(targetMessageId);
    setNewMessageCount(0);
    virtuosoRef.current?.scrollToIndex({
      align: "center",
      behavior: "auto",
      index,
    });
    onTargetReached?.(targetMessageId);

    const timeout = window.setTimeout(() => {
      setHighlightedMessageId((current) =>
        current === targetMessageId ? null : current,
      );
    }, 2_000);

    return () => {
      window.clearTimeout(timeout);
    };
  }, [isLoading, messages, onTargetReached, targetMessageId]);

  return (
    <TooltipProvider delayDuration={200}>
      <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <div
          className={cn(
            "absolute inset-0 min-h-0 min-w-0 px-4 pt-1 sm:px-6",
            channelChrome.contentPadding,
          )}
        >
          {showMessageList ? (
            <VirtualizedTimelineMessageList
              agentPubkeys={agentPubkeys}
              atBottomStateChange={handleAtBottomStateChange}
              bottomFooterClassName={hasComposerOverlay ? "h-24" : "h-4"}
              channelId={channelId}
              channelName={channelName}
              channelType={channelType}
              currentPubkey={currentPubkey}
              followOutput={() =>
                shouldStickToBottomRef.current ? "auto" : false
              }
              followThreadById={followThreadById}
              hasOlderMessages={hasOlderMessages}
              highlightedMessageId={highlightedMessageId}
              isFetchingOlder={isFetchingOlder}
              isFollowingThreadById={isFollowingThreadById}
              messageFooters={messageFooters}
              messages={messages}
              onDelete={onDelete}
              onEdit={onEdit}
              onMarkUnread={onMarkUnread}
              onReply={onReply}
              onStartReached={handleStartReached}
              isSendingVideoReviewComment={isSendingVideoReviewComment}
              onSendVideoReviewComment={onSendVideoReviewComment}
              onToggleReaction={onToggleReaction}
              personaLookup={personaLookup}
              profiles={profiles}
              scrollerRef={(element) => {
                setScrollerRef(element);
                if (element) {
                  element.dataset.scrollRestorationId = scrollRestorationId;
                }
              }}
              searchActiveMessageId={searchActiveMessageId}
              searchMatchingMessageIds={searchMatchingMessageIds}
              searchQuery={searchQuery}
              topHeader={
                showDirectMessageIntro ? (
                  <DirectMessageIntroCard
                    directMessageIntro={directMessageIntro}
                  />
                ) : showChannelIntro ? (
                  <ChannelIntroCard channelIntro={channelIntro} />
                ) : null
              }
              unfollowThreadById={unfollowThreadById}
              virtuosoRef={virtuosoRef}
            />
          ) : (
            <div
              className={cn(
                "h-full overflow-y-auto overflow-x-hidden overscroll-contain [overflow-anchor:none]",
                hasComposerOverlay ? "pb-24" : "pb-4",
              )}
              data-scroll-restoration-id={scrollRestorationId}
              data-testid="message-timeline"
              ref={scrollContainerRef}
            >
              <div
                className={cn(
                  "flex min-h-full w-full flex-col gap-2",
                  (showIntro || showGenericEmpty) && "min-h-full",
                )}
              >
                {isLoading ? <TimelineSkeleton /> : null}

                {showDirectMessageIntro ? (
                  <DirectMessageIntroCard
                    directMessageIntro={directMessageIntro}
                  />
                ) : null}

                {showChannelIntro ? (
                  <ChannelIntroCard channelIntro={channelIntro} />
                ) : null}

                {showGenericEmpty ? (
                  <div
                    className="mt-auto rounded-3xl border border-dashed border-border/80 bg-card/70 px-6 py-10 text-center shadow-xs"
                    data-testid="message-empty"
                  >
                    <p className="text-base font-semibold tracking-tight">
                      {emptyTitle}
                    </p>
                    <p className="mt-2 text-sm text-muted-foreground">
                      {emptyDescription}
                    </p>
                  </div>
                ) : null}
              </div>
            </div>
          )}

          {isFetchingOlder && showMessageList ? (
            <div className="pointer-events-none absolute inset-x-0 top-3 z-20 flex justify-center">
              <div className="rounded-full border border-border/70 bg-background/85 p-1 shadow-xs backdrop-blur-sm">
                <Spinner className="h-4 w-4 text-muted-foreground" />
              </div>
            </div>
          ) : null}
        </div>

        {!isAtBottom ? (
          <div
            className={cn(
              "pointer-events-none absolute inset-x-0 z-20 flex justify-center px-4",
              hasComposerOverlay ? "bottom-36" : "bottom-4",
            )}
          >
            <Button
              className="pointer-events-auto h-7 min-h-7 gap-1.5 rounded-full border-border/50 bg-background/85 px-2.5 text-[11px] font-medium text-muted-foreground shadow-xs backdrop-blur-sm hover:bg-muted/70 hover:text-foreground [&_svg]:size-3.5"
              data-testid="message-scroll-to-latest"
              onClick={() => {
                scrollToBottom("smooth");
              }}
              size="sm"
              type="button"
              variant="outline"
            >
              <ArrowDown aria-hidden />
              {newMessageCount > 0
                ? `${newMessageCount} new message${newMessageCount === 1 ? "" : "s"}`
                : "Jump to latest"}
            </Button>
          </div>
        ) : null}
      </div>
    </TooltipProvider>
  );
});

function DirectMessageIntroCard({
  directMessageIntro,
}: {
  directMessageIntro: {
    avatarUrl: string | null;
    displayName: string;
  };
}) {
  return (
    <div
      className="mb-0.5 mt-auto flex w-full flex-col items-start px-3 py-2 text-left"
      data-testid="message-dm-intro"
    >
      <ProfileAvatar
        avatarUrl={directMessageIntro.avatarUrl}
        className="h-[60px] w-[60px] text-base"
        iconClassName="h-6 w-6"
        label={directMessageIntro.displayName}
        testId="message-dm-intro-avatar"
      />
      <p className="mt-4 max-w-full truncate text-xl font-semibold leading-7 tracking-tight text-foreground">
        {directMessageIntro.displayName}
      </p>
      <p className="mt-1 max-w-full truncate whitespace-nowrap text-sm leading-5 text-muted-foreground">
        This is the beginning of your direct message with{" "}
        <span className="font-medium text-foreground">
          {directMessageIntro.displayName}
        </span>
        .
      </p>
    </div>
  );
}

function ChannelIntroCard({ channelIntro }: { channelIntro: ChannelIntro }) {
  return (
    <div
      className="mb-0.5 mt-auto flex w-full max-w-2xl flex-col items-start px-3 py-2 text-left"
      data-testid="message-channel-intro"
    >
      <div
        className="flex h-[60px] w-[60px] items-center justify-center rounded-2xl border border-border/70 bg-muted/40 text-muted-foreground"
        data-testid="message-channel-intro-icon"
      >
        {channelIntro.icon ?? <Hash aria-hidden className="h-7 w-7" />}
      </div>
      <p className="mt-4 max-w-full truncate text-xl font-semibold leading-7 tracking-tight text-foreground">
        #{channelIntro.channelName}
      </p>
      <p className="mt-1 max-w-full text-sm leading-5 text-muted-foreground">
        This is the beginning of the{" "}
        <span className="font-medium text-foreground">
          {channelIntro.channelKindLabel}
        </span>
        .
      </p>
      {channelIntro.description ? (
        <p className="mt-2 max-w-xl text-sm leading-5 text-muted-foreground">
          {channelIntro.description}
        </p>
      ) : null}
      {channelIntro.actions?.length ? (
        <div className="mt-4 flex max-w-full flex-nowrap gap-3 overflow-x-auto pb-1">
          {channelIntro.actions.map((action) => {
            const hasDescription = Boolean(action.description);

            return (
              <button
                className={cn(
                  "flex shrink-0 border border-border/70 bg-background/70 text-left transition-colors hover:bg-muted/60 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
                  hasDescription
                    ? "h-56 w-[13.75rem] flex-col rounded-2xl p-4"
                    : "h-28 w-64 flex-col rounded-xl p-4",
                )}
                data-testid={action.testId}
                key={action.label}
                onClick={action.onClick}
                type="button"
              >
                <span
                  className={cn(
                    "flex shrink-0 items-center justify-center rounded-full bg-muted/70 text-muted-foreground",
                    hasDescription
                      ? "h-12 w-12 [&_svg]:h-6 [&_svg]:w-6"
                      : "h-10 w-10 [&_svg]:h-5 [&_svg]:w-5",
                  )}
                  data-testid={
                    action.testId ? `${action.testId}-icon` : undefined
                  }
                >
                  {action.icon}
                </span>
                <span className="mt-auto min-w-0">
                  <span
                    className="block whitespace-normal break-words text-base font-medium leading-6 text-foreground"
                    data-testid={
                      action.testId ? `${action.testId}-title` : undefined
                    }
                  >
                    {action.label}
                  </span>
                  {action.description ? (
                    <span
                      className="mt-1 block whitespace-normal break-words text-sm leading-5 text-muted-foreground"
                      data-testid={
                        action.testId
                          ? `${action.testId}-description`
                          : undefined
                      }
                    >
                      {action.description}
                    </span>
                  ) : null}
                </span>
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
