import * as React from "react";

import { useKnownAgentPubkeys } from "@/features/agents/useKnownAgentPubkeys";
import type { InboxContextMessage } from "@/features/home/lib/inbox";
import { toTimelineMessage } from "@/features/home/lib/inboxViewHelpers";
import { formatTimeWithoutDayPeriod } from "@/features/messages/lib/dateFormatters";
import type { TimelineMessage } from "@/features/messages/types";
import { getConfigNudgeAuthorPubkey } from "@/features/messages/ui/configNudgeAuthPubkey";
import { MessageActionBar } from "@/features/messages/ui/MessageActionBar";
import { MessageReactions } from "@/features/messages/ui/MessageReactions";
import { useReactionHandler } from "@/features/messages/ui/useReactionHandler";
import { useMessageEmoji } from "@/features/messages/lib/useMessageEmoji";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Markdown } from "@/shared/ui/markdown";
import { UserAvatar } from "@/shared/ui/UserAvatar";

export type InboxDisplayMessage = InboxContextMessage & {
  depth: number;
};

type InboxMessageRowProps = {
  agentPubkeys?: ReadonlySet<string>;
  canReply: boolean;
  /** Channel UUID for "Copy link" — passed straight through to MessageActionBar. */
  channelId?: string | null;
  isContinuation?: boolean;
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
  agentPubkeys,
  canReply,
  channelId = null,
  isContinuation = false,
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
  // "Is this pubkey an agent" = the workspace-scoped baseline every surface
  // shares plus this surface's extras passed via `agentPubkeys` (HomeView
  // folds feed-profile `isAgent` flags in). Mirrors MessageRow's predicate.
  const knownAgentPubkeys = useKnownAgentPubkeys();
  const isKnownAgentPubkey = React.useCallback(
    (pubkey: string) => {
      const normalized = normalizePubkey(pubkey);
      return (
        knownAgentPubkeys.has(normalized) ||
        agentPubkeys?.has(normalized) === true
      );
    },
    [agentPubkeys, knownAgentPubkeys],
  );
  const isAuthorAgent = isKnownAgentPubkey(message.authorPubkey);
  const profileRole = isAuthorAgent ? "bot" : undefined;
  const hoverTimestampLabel = formatTimeWithoutDayPeriod(
    message.timeLabel ?? message.fullTimestampLabel,
  );

  return (
    <div className="relative px-2">
      {message.isSelected ? (
        <div
          aria-hidden="true"
          className={cn(
            "pointer-events-none absolute inset-x-3 inset-y-1 rounded-2xl transition-opacity duration-1000",
            isFocusHighlightVisible
              ? "bg-primary/[0.07] opacity-100"
              : "bg-primary/[0.07] opacity-0",
          )}
        />
      ) : null}
      <article
        className={cn(
          "group/message relative z-10 mx-1 flex gap-2.5 rounded-2xl px-2 py-1 transition-colors hover:bg-muted/50 focus-within:bg-muted/50",
          isContinuation ? "items-center" : "items-start",
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

        {isContinuation ? (
          <div
            aria-hidden="true"
            className="flex w-9 shrink-0 self-stretch items-start justify-end pt-0.5"
            title={message.fullTimestampLabel}
          >
            <p className="shrink-0 cursor-default whitespace-nowrap text-xs font-normal leading-4 tabular-nums text-muted-foreground/55 opacity-0 transition-opacity group-hover/message:opacity-100 group-focus-within/message:opacity-100">
              {hoverTimestampLabel}
            </p>
          </div>
        ) : (
          <div className="relative shrink-0">
            <UserProfilePopover
              botIdenticonValue={message.authorLabel}
              pubkey={message.authorPubkey}
              role={profileRole}
              triggerElement="span"
            >
              <span className="inline-flex shrink-0 rounded-full focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring">
                <UserAvatar
                  avatarUrl={message.avatarUrl}
                  className="h-9 w-9 shrink-0"
                  displayName={message.authorLabel}
                  size="md"
                />
              </span>
            </UserProfilePopover>
          </div>
        )}

        <div className="min-w-0 flex-1">
          {isContinuation ? null : (
            <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0">
              <UserProfilePopover
                botIdenticonValue={message.authorLabel}
                pubkey={message.authorPubkey}
                role={profileRole}
                triggerElement="span"
              >
                <span className="block max-w-full truncate rounded text-sm font-semibold text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring">
                  {message.authorLabel}
                </span>
              </UserProfilePopover>
              <p className="shrink-0 text-xs font-normal tabular-nums text-muted-foreground/55">
                {message.fullTimestampLabel}
              </p>
            </div>
          )}

          <div className={isContinuation ? "mt-0" : "mt-0.5"}>
            <Markdown
              className={cn(
                "max-w-full text-left text-sm text-foreground",
                emojiOnly &&
                  "text-4xl leading-tight [&_p]:leading-tight [&_img[data-custom-emoji]]:h-[1.45em] [&_img[data-custom-emoji]]:align-middle [&_button:has(img[data-custom-emoji])]:align-middle",
              )}
              // Only pass the author pubkey for agent-authored messages so
              // config-nudge cards can authenticate the sender. Uses the
              // raw event signer (signerPubkey) — not the tag-attributed
              // display author — to prevent actor/p-tag spoofing.
              configNudgeAuthorPubkey={getConfigNudgeAuthorPubkey(
                timelineMessage,
                isKnownAgentPubkey,
              )}
              content={message.content}
              customEmoji={customEmoji}
              mentionNames={message.mentionNames}
              mentionPubkeysByName={message.mentionPubkeysByName}
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
