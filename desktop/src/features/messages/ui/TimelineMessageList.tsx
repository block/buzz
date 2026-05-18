import * as React from "react";

import {
  formatDayHeading,
  isSameDay,
} from "@/features/messages/lib/dateFormatters";
import { buildMainTimelineEntries } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";
import { DayDivider } from "./DayDivider";
import { MessageRow } from "./MessageRow";
import { MessageThreadSummaryRow } from "./MessageThreadSummaryRow";
import { SystemMessageRow } from "./SystemMessageRow";

type TimelineMessageListProps = {
  activeReplyTargetId?: string | null;
  channelId?: string | null;
  currentPubkey?: string;
  highlightedMessageId?: string | null;
  messageFooters?: Record<string, React.ReactNode>;
  messages: TimelineMessage[];
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onReply?: (message: TimelineMessage) => void;
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
  activeReplyTargetId = null,
  channelId,
  currentPubkey,
  highlightedMessageId = null,
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
      currentDayGroup?.elements.push(
        <div key={message.id} className="flex flex-col gap-0">
          <MessageRow
            activeReplyTargetId={activeReplyTargetId}
            channelId={channelId}
            highlighted={message.id === highlightedMessageId}
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
            activeReplyTargetId={activeReplyTargetId}
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
