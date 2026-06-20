import * as React from "react";

import type { InboxContextMessage } from "@/features/home/lib/inbox";
import type { TimelineMessage } from "@/features/messages/types";
import { MessageActionBar } from "@/features/messages/ui/MessageActionBar";
import { MessageReactions } from "@/features/messages/ui/MessageReactions";
import { useReactionHandler } from "@/features/messages/ui/useReactionHandler";
import { useMessageEmoji } from "@/features/messages/lib/useMessageEmoji";
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
    tags: message.tags,
    time: message.fullTimestampLabel,
  };
}

type InboxMessageRowProps = {
  canReply: boolean;
  /** Channel UUID for "Copy link" — passed straight through to MessageActionBar. */
  channelId?: string | null;
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
  canReply,
  channelId = null,
  isFocusHighlightVisible,
  message,
  onSelectReplyTarget,
  onToggleReaction,
}: InboxMessageRowProps) {
  const timelineMessage = React.useMemo(
    () => toTimelineMessage(message),
    [message],
  );
  const { customEmoji, emojiOnly } = useMessageEmoji(
    message.content,
    message.tags,
  );
  const [badgeBurstEmoji, setBadgeBurstEmoji] = React.useState<string | null>(
    null,
  );
  const {
    reactions,
    canToggle: canToggleReactions,
    pending: reactionPending,
    errorMessage: reactionErrorMessage,
    select: handleReactionSelect,
  } = useReactionHandler(timelineMessage, onToggleReaction);

  return (
    <div className="relative px-5 py-2">
      {message.isSelected ? (
        <div
          aria-hidden="true"
          className={cn(
            "pointer-events-none absolute inset-x-0 inset-y-1 transition-opacity duration-1000",
            isFocusHighlightVisible
              ? "bg-primary/[0.07] opacity-100"
              : "bg-primary/[0.07] opacity-0",
          )}
        />
      ) : null}
      <article
        className={cn(
          "group/message relative flex items-start gap-2.5 px-0 py-0",
          !message.isSelected && "hover:bg-muted/20",
        )}
        data-testid={
          message.isSelected
            ? "home-inbox-selected-message"
            : "home-inbox-context-message"
        }
      >
        {canReply || canToggleReactions ? (
          <div className="absolute right-2 top-1 z-10 sm:top-0 sm:-translate-y-1/2">
            <MessageActionBar
              channelId={channelId}
              message={timelineMessage}
              onReactionSelect={
                canToggleReactions ? handleReactionSelect : undefined
              }
              onReactionBadgeBurstRequest={
                reactionPending ? undefined : setBadgeBurstEmoji
              }
              onReply={
                canReply ? () => onSelectReplyTarget(message) : undefined
              }
              reactionErrorMessage={reactionErrorMessage}
              reactions={reactions}
            />
          </div>
        ) : null}

        <div className="relative shrink-0">
          <UserAvatar
            avatarUrl={message.avatarUrl}
            className="h-8 w-8 shrink-0"
            displayName={message.authorLabel}
            size="md"
          />
        </div>

        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0">
            <p className="truncate text-sm font-semibold text-foreground">
              {message.authorLabel}
            </p>
            <p className="shrink-0 text-xs font-normal tabular-nums text-muted-foreground/55">
              {message.fullTimestampLabel}
            </p>
          </div>

          <div className="mt-0.5">
            <Markdown
              className={cn(
                "max-w-full text-left text-sm text-foreground",
                emojiOnly &&
                  "text-4xl leading-tight [&_p]:leading-tight [&_img[data-custom-emoji]]:h-[1.45em] [&_img[data-custom-emoji]]:align-middle [&_button:has(img[data-custom-emoji])]:align-middle",
              )}
              content={message.content}
              customEmoji={customEmoji}
              mentionNames={message.mentionNames}
            />
            <MessageReactions
              canToggle={canToggleReactions}
              messageId={message.id}
              onSelect={(emoji) => {
                void handleReactionSelect(emoji);
              }}
              burstEmojiOnRender={badgeBurstEmoji}
              onBurstEmojiRendered={(emoji) => {
                setBadgeBurstEmoji((current) =>
                  current === emoji ? null : current,
                );
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
