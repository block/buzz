import * as React from "react";
import { ClipboardPlus } from "lucide-react";

import type { AgentConversationMarker } from "@/features/agents/agentConversations";
import {
  formatDayHeading,
  formatTime,
} from "@/features/messages/lib/dateFormatters";
import { isBroadcastReply } from "@/features/messages/lib/threading";
import type { TimelineMessage } from "@/features/messages/types";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type { Channel } from "@/shared/api/types";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

type ChannelTaskItem = {
  marker: AgentConversationMarker;
  message: TimelineMessage | null;
  threadMessage: TimelineMessage | null;
};

type ChannelTasksViewProps = {
  activeChannel: Channel | null;
  agentConversationMarkers?: readonly AgentConversationMarker[];
  currentPubkey?: string;
  fetchOlder?: () => Promise<void>;
  hasOlderMessages?: boolean;
  isFetchingOlder?: boolean;
  isTimelineLoading?: boolean;
  messages: readonly TimelineMessage[];
  onOpenAgentConversation?: (
    message: TimelineMessage,
    options?: { publishMarker?: boolean },
  ) => void;
  onGoToTaskMessage?: (
    marker: AgentConversationMarker,
    message: TimelineMessage,
    threadMessage: TimelineMessage,
  ) => void;
  profiles?: UserProfileLookup;
  scrollContainerRef: React.RefObject<HTMLDivElement | null>;
};

function formatTaskStartedAt(unixSeconds: number): string {
  return `${formatDayHeading(unixSeconds)} at ${formatTime(unixSeconds)}`;
}

function ChannelTaskRow({
  currentPubkey,
  marker,
  message,
  onOpenAgentConversation,
  onGoToTaskMessage,
  profiles,
  threadMessage,
}: {
  currentPubkey?: string;
  marker: AgentConversationMarker;
  message: TimelineMessage | null;
  onOpenAgentConversation?: (
    message: TimelineMessage,
    options?: { publishMarker?: boolean },
  ) => void;
  onGoToTaskMessage?: (
    marker: AgentConversationMarker,
    message: TimelineMessage,
    threadMessage: TimelineMessage,
  ) => void;
  profiles?: UserProfileLookup;
  threadMessage: TimelineMessage | null;
}) {
  const startedAt = marker.startedAt || marker.createdAt;
  const starterName = resolveUserLabel({
    currentPubkey,
    profiles,
    pubkey: marker.starterPubkey,
  });

  return (
    <article
      className="group/task mx-1 min-w-0 overflow-hidden rounded-lg border border-border/70 bg-muted/35 transition-colors hover:bg-muted/45 focus-within:bg-muted/45"
      data-agent-conversation-id={marker.eventId}
      data-testid="channel-task-row"
    >
      <div className="flex min-w-0 items-center gap-3 px-3 py-2">
        <div
          aria-hidden
          className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-background p-2.5 text-muted-foreground shadow-xs ring-1 ring-border/60"
        >
          <ClipboardPlus className="size-5" />
        </div>
        <div className="min-w-0 flex-1">
          <p
            className="truncate text-sm font-medium text-foreground"
            title={marker.title}
          >
            {marker.title}
          </p>
          <p className="mt-0.5 truncate text-xs text-muted-foreground">
            {starterName} · {formatTaskStartedAt(startedAt)}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2 opacity-0 transition-opacity group-hover/task:opacity-100 group-focus-within/task:opacity-100">
          <Button
            className="h-8 rounded-lg bg-transparent px-3 text-xs font-medium text-foreground shadow-none hover:bg-secondary hover:text-secondary-foreground"
            data-testid="channel-task-go-to-thread"
            disabled={!onGoToTaskMessage || !message || !threadMessage}
            onClick={() => {
              if (message && threadMessage) {
                onGoToTaskMessage?.(marker, message, threadMessage);
              }
            }}
            title="Go to source message in channel"
            type="button"
            variant="ghost"
          >
            Go to message
          </Button>
          <Button
            className="h-8 rounded-lg px-3 text-xs font-medium"
            data-testid="channel-task-open"
            disabled={!onOpenAgentConversation || !message}
            onClick={() => {
              if (message) {
                onOpenAgentConversation?.(message, { publishMarker: false });
              }
            }}
            type="button"
            variant="outline"
          >
            Open
          </Button>
        </div>
      </div>
    </article>
  );
}

export function ChannelTasksView({
  activeChannel,
  agentConversationMarkers,
  currentPubkey,
  messages,
  fetchOlder,
  hasOlderMessages,
  isFetchingOlder,
  isTimelineLoading,
  onOpenAgentConversation,
  onGoToTaskMessage,
  profiles,
  scrollContainerRef,
}: ChannelTasksViewProps) {
  const loadOlderRef = React.useRef<HTMLDivElement | null>(null);
  const messageById = React.useMemo(
    () => new Map(messages.map((message) => [message.id, message])),
    [messages],
  );
  const channelTaskMarkers = React.useMemo(() => {
    const channelId = activeChannel?.id ?? null;

    return (agentConversationMarkers ?? []).filter(
      (marker) => !channelId || marker.channelId === channelId,
    );
  }, [activeChannel?.id, agentConversationMarkers]);
  const canLoadOlderTasks = Boolean(
    fetchOlder &&
      hasOlderMessages &&
      !isTimelineLoading &&
      (messages.length > 0 || channelTaskMarkers.length > 0),
  );
  const canAutoLoadOlderTasks =
    canLoadOlderTasks && channelTaskMarkers.length > 0;
  const handleLoadOlderTasks = React.useCallback(() => {
    if (!fetchOlder || isFetchingOlder) {
      return;
    }

    void fetchOlder();
  }, [fetchOlder, isFetchingOlder]);

  React.useEffect(() => {
    if (!canAutoLoadOlderTasks || isFetchingOlder) {
      return;
    }

    const root = scrollContainerRef.current;
    const target = loadOlderRef.current;
    if (!root || !target) {
      return;
    }

    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry?.isIntersecting) {
          handleLoadOlderTasks();
        }
      },
      { root, rootMargin: "160px 0px" },
    );
    observer.observe(target);
    return () => observer.disconnect();
  }, [
    canAutoLoadOlderTasks,
    handleLoadOlderTasks,
    isFetchingOlder,
    scrollContainerRef,
  ]);

  const taskItems = React.useMemo<ChannelTaskItem[]>(() => {
    return channelTaskMarkers
      .map((marker) => {
        const message = messageById.get(marker.agentReplyId) ?? null;
        const resolvedThreadMessage =
          messageById.get(marker.threadRootMessageId ?? "") ??
          messageById.get(marker.threadRootId) ??
          null;
        const isBroadcastTaskSource = message
          ? isBroadcastReply(message.tags ?? [])
          : false;
        const threadMessage =
          resolvedThreadMessage ??
          (marker.threadRootId === marker.agentReplyId || isBroadcastTaskSource
            ? message
            : null);
        return {
          marker,
          message,
          threadMessage,
        };
      })
      .sort(
        (left, right) =>
          (right.marker.startedAt || right.marker.createdAt) -
            (left.marker.startedAt || left.marker.createdAt) ||
          right.marker.eventId.localeCompare(left.marker.eventId),
      );
  }, [channelTaskMarkers, messageById]);
  const olderTasksLoader = canLoadOlderTasks ? (
    <div className="flex justify-center px-3 pt-2" ref={loadOlderRef}>
      <Button
        className="h-8 rounded-lg px-3 text-xs font-medium"
        disabled={isFetchingOlder}
        onClick={handleLoadOlderTasks}
        type="button"
        variant="ghost"
      >
        {isFetchingOlder ? "Loading older tasks..." : "Load older tasks"}
      </Button>
    </div>
  ) : null;

  return (
    <div
      className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
      data-testid="channel-tasks-view"
    >
      <div
        className="absolute inset-0 overflow-y-auto overflow-x-hidden overscroll-none px-2 pb-8 pt-1 [overflow-anchor:none]"
        ref={scrollContainerRef}
      >
        <div
          className={cn(
            "mx-auto flex w-full max-w-4xl flex-col gap-6 px-3",
            channelChrome.contentPadding,
          )}
        >
          {taskItems.length === 0 ? (
            <>
              <div
                className="mt-10 rounded-3xl border border-dashed border-border/80 bg-card/70 px-6 py-10 text-center shadow-xs"
                data-testid="channel-tasks-empty"
              >
                <div className="mx-auto flex size-12 items-center justify-center rounded-2xl border border-border/70 bg-muted/40 text-muted-foreground">
                  <ClipboardPlus className="size-5" />
                </div>
                <p className="mt-4 text-base font-semibold tracking-tight">
                  No tasks yet
                </p>
                <p className="mx-auto mt-2 max-w-sm text-sm leading-5 text-muted-foreground">
                  New tasks will appear here when an agent conversation is
                  opened from this channel.
                </p>
              </div>
              {olderTasksLoader}
            </>
          ) : (
            <div className="mt-3 flex min-w-0 flex-col gap-2">
              {taskItems.map(({ marker, message, threadMessage }) => (
                <ChannelTaskRow
                  currentPubkey={currentPubkey}
                  key={marker.eventId}
                  marker={marker}
                  message={message}
                  onOpenAgentConversation={onOpenAgentConversation}
                  onGoToTaskMessage={onGoToTaskMessage}
                  profiles={profiles}
                  threadMessage={threadMessage}
                />
              ))}
              {olderTasksLoader}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
