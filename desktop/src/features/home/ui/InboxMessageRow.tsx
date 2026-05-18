import * as React from "react";

import type { InboxContextMessage } from "@/features/home/lib/inbox";
import type { TimelineMessage } from "@/features/messages/types";
import { MessageActionBar } from "@/features/messages/ui/MessageActionBar";
import { MessageReactions } from "@/features/messages/ui/MessageReactions";
import { useReactionHandler } from "@/features/messages/ui/useReactionHandler";
import { cn } from "@/shared/lib/cn";
import { Markdown } from "@/shared/ui/markdown";
import { UserAvatar } from "@/shared/ui/UserAvatar";

export type InboxDisplayMessage = InboxContextMessage & {
  depth: number;
};

function toTimelineMessage(message: InboxDisplayMessage): TimelineMessage {
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

type InboxMessageRowProps = {
  activeReplyTargetId: string | null;
  canReply: boolean;
  isFocusHighlightVisible: boolean;
  message: InboxDisplayMessage;
  onSelectReplyTarget: (message: InboxDisplayMessage) => void;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
};

export function InboxMessageRow({
  activeReplyTargetId,
  canReply,
  isFocusHighlightVisible,
  message,
  onSelectReplyTarget,
  onToggleReaction,
}: InboxMessageRowProps) {
  const timelineMessage = React.useMemo(
    () => toTimelineMessage(message),
    [message],
  );
  const {
    reactions,
    canToggle: canToggleReactions,
    pending: reactionPending,
    errorMessage: reactionErrorMessage,
    select: handleReactionSelect,
  } = useReactionHandler(timelineMessage, onToggleReaction);

  return (
    <div className="px-6 py-2">
      <article
        className={cn(
          "group/message relative flex items-start gap-2.5 px-2 py-1",
          !message.isSelected && "hover:bg-muted/20",
        )}
        data-testid={
          message.isSelected
            ? "home-inbox-selected-message"
            : "home-inbox-context-message"
        }
      >
        {message.isSelected ? (
          <div
            aria-hidden="true"
            className={cn(
              "pointer-events-none absolute -inset-x-2 -inset-y-1 rounded-xl transition-opacity duration-1000",
              isFocusHighlightVisible
                ? "bg-primary/[0.07] opacity-100"
                : "bg-primary/[0.07] opacity-0",
            )}
          />
        ) : null}

        {canReply || canToggleReactions ? (
          <div className="absolute right-2 top-1 z-10">
            <MessageActionBar
              activeReplyTargetId={activeReplyTargetId}
              message={timelineMessage}
              onReactionSelect={
                canToggleReactions ? handleReactionSelect : undefined
              }
              onReply={
                canReply ? () => onSelectReplyTarget(message) : undefined
              }
              reactionErrorMessage={reactionErrorMessage}
              reactionPending={reactionPending}
              reactions={reactions}
            />
          </div>
        ) : null}

        <UserAvatar
          avatarUrl={message.avatarUrl}
          className="!h-9 !w-9 shrink-0"
          displayName={message.authorLabel}
          size="md"
        />

        <div className="-mt-1 min-w-0 flex-1">
          <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0">
            <p className="truncate text-sm font-semibold leading-none tracking-tight text-foreground">
              {message.authorLabel}
            </p>
            <p className="shrink-0 text-xs font-normal leading-none tabular-nums text-muted-foreground/55">
              {message.fullTimestampLabel}
            </p>
            {message.isSelected ? (
              <span className="text-[10px] font-semibold uppercase leading-none tracking-[0.14em] text-muted-foreground/70">
                Inbox item
              </span>
            ) : null}
          </div>

          <div className="mt-1">
            <Markdown
              className="max-w-full text-left text-sm text-foreground"
              content={message.content}
              mentionNames={message.mentionNames}
              tight
            />
            <MessageReactions
              canToggle={canToggleReactions}
              messageId={message.id}
              onSelect={(emoji) => {
                void handleReactionSelect(emoji);
              }}
              pending={reactionPending}
              reactions={reactions}
            />
            {reactionErrorMessage ? (
              <p className="mt-1.5 text-xs text-destructive">
                {reactionErrorMessage}
              </p>
            ) : null}
          </div>
        </div>
      </article>
    </div>
  );
}
