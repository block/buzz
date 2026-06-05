import * as React from "react";

import {
  formatDayHeading,
  isSameDay,
} from "@/features/messages/lib/dateFormatters";
import { buildMainTimelineEntries } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { DayDivider } from "./DayDivider";
import { MessageRow } from "./MessageRow";
import { MessageThreadSummaryRow } from "./MessageThreadSummaryRow";
import { SystemMessageRow } from "./SystemMessageRow";

type TimelineMessageListProps = {
  channelId?: string | null;
  currentPubkey?: string;
  followThreadById?: (rootId: string) => void;
  highlightedMessageId?: string | null;
  isFollowingThreadById?: (rootId: string) => boolean;
  messageFooters?: Record<string, React.ReactNode>;
  messages: TimelineMessage[];
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onReply?: (message: TimelineMessage) => void;
  unfollowThreadById?: (rootId: string) => void;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  /** Map from lowercase pubkey → persona display name for bot members. */
  personaLookup?: Map<string, string>;
  profiles?: UserProfileLookup;
  /** The message ID of the currently active find-in-channel match. */
  searchActiveMessageId?: string | null;
  /** Set of message IDs that match the current find-in-channel query. */
  searchMatchingMessageIds?: Set<string>;
  /** The current find-in-channel query string. */
  searchQuery?: string;
};

export const TimelineMessageList = React.memo(function TimelineMessageList({
  channelId,
  currentPubkey,
  followThreadById,
  highlightedMessageId = null,
  isFollowingThreadById,
  messageFooters,
  messages,
  onDelete,
  onEdit,
  onMarkUnread,
  onReply,
  onToggleReaction,
  personaLookup,
  profiles,
  searchActiveMessageId = null,
  searchMatchingMessageIds,
  searchQuery,
  unfollowThreadById,
}: TimelineMessageListProps) {
  const entries = React.useMemo(
    () => buildMainTimelineEntries(messages),
    [messages],
  );
  const dayGroups: Array<{
    key: string;
    label: string;
    elements: React.ReactNode[];
  }> = [];
  let currentDayGroup: (typeof dayGroups)[number] | null = null;

  for (let i = 0; i < entries.length; i++) {
    const { message, summary } = entries[i];
    const prev = i > 0 ? entries[i - 1]?.message : null;

    if (!prev || !isSameDay(prev.createdAt, message.createdAt)) {
      currentDayGroup = {
        key: `day-${message.createdAt}`,
        label: formatDayHeading(message.createdAt),
        elements: [],
      };
      dayGroups.push(currentDayGroup);
    }

    if (message.kind === KIND_SYSTEM_MESSAGE) {
      const footer = messageFooters?.[message.id] ?? null;
      currentDayGroup?.elements.push(
        <div key={message.id} className="flex flex-col gap-1">
          <SystemMessageRow
            message={message}
            currentPubkey={currentPubkey}
            onToggleReaction={onToggleReaction}
            personaLookup={personaLookup}
            profiles={profiles}
          />
          {footer}
        </div>,
      );
    } else if (summary && onReply) {
      const footer = messageFooters?.[message.id] ?? null;
      const isHighlighted = message.id === highlightedMessageId;
      currentDayGroup?.elements.push(
        <div
          key={message.id}
          className={cn(
            "group/message relative -mx-1 flex flex-col gap-0 rounded-2xl px-1 py-1 transition-colors hover:bg-muted/50 focus-within:bg-muted/50",
            isHighlighted &&
              "-mx-4 px-4 before:absolute before:-inset-y-1.5 before:inset-x-0 before:animate-[route-target-highlight-fade_2s_ease-out_forwards] before:bg-primary/10 before:content-[''] motion-reduce:before:animate-none sm:-mx-6 sm:px-6",
          )}
        >
          <MessageRow
            channelId={channelId}
            highlighted={false}
            hoverBackground={false}
            isFollowingThread={
              isFollowingThreadById
                ? isFollowingThreadById(message.id)
                : undefined
            }
            message={message}
            onDelete={
              onDelete && currentPubkey && message.pubkey === currentPubkey
                ? onDelete
                : undefined
            }
            onEdit={
              onEdit && currentPubkey && message.pubkey === currentPubkey
                ? onEdit
                : undefined
            }
            onFollowThread={
              followThreadById ? () => followThreadById(message.id) : undefined
            }
            onMarkUnread={onMarkUnread}
            onToggleReaction={onToggleReaction}
            onReply={onReply}
            onUnfollowThread={
              unfollowThreadById
                ? () => unfollowThreadById(message.id)
                : undefined
            }
            profiles={profiles}
          />
          <MessageThreadSummaryRow
            depth={message.depth}
            message={message}
            onOpenThread={onReply}
            summary={summary}
          />
          {footer}
        </div>,
      );
    } else {
      const isSearchMatch = searchMatchingMessageIds?.has(message.id) ?? false;
      const isSearchActive = message.id === searchActiveMessageId;
      const footer = messageFooters?.[message.id] ?? null;

      currentDayGroup?.elements.push(
        <div key={message.id} className="flex flex-col gap-1">
          <MessageRow
            channelId={channelId}
            highlighted={message.id === highlightedMessageId || isSearchActive}
            message={message}
            onDelete={
              onDelete && currentPubkey && message.pubkey === currentPubkey
                ? onDelete
                : undefined
            }
            onEdit={
              onEdit && currentPubkey && message.pubkey === currentPubkey
                ? onEdit
                : undefined
            }
            onMarkUnread={onMarkUnread}
            onToggleReaction={onToggleReaction}
            onReply={onReply}
            profiles={profiles}
            searchQuery={isSearchMatch ? searchQuery : undefined}
          />
          {footer}
        </div>,
      );
    }
  }

  return dayGroups.map((group) => (
    <section className="flex flex-col gap-2.5" key={group.key}>
      <DayDivider label={group.label} />
      {group.elements}
    </section>
  ));
});
