import type { TimelineMessage } from "@/features/messages/types";
import { isBroadcastReply } from "@/features/messages/lib/threading";

type ThreadPanelData = {
  threadHead: TimelineMessage | null;
  totalReplyCount: number;
  visibleReplies: MainTimelineEntry[];
  replyTargetMessage: TimelineMessage | null;
};

export type TimelineThreadSummaryParticipant = {
  id: string;
  author: string;
  avatarUrl: string | null;
};

export type TimelineThreadSummary = {
  threadHeadId: string;
  replyCount: number;
  lastReplyAt: number | null;
  participants: TimelineThreadSummaryParticipant[];
};

export type MainTimelineEntry = {
  message: TimelineMessage;
  summary: TimelineThreadSummary | null;
};

type ThreadDescendantStats = {
  descendantCount: number;
  lastReplyAt: number | null;
  recentParticipantsNewestFirst: TimelineThreadSummaryParticipant[];
};

const MAX_SUMMARY_PARTICIPANTS = 3;

function normalizeHeadMessage(message: TimelineMessage): TimelineMessage {
  return {
    ...message,
    depth: 0,
  };
}

function normalizeInlineReplyMessage(
  message: TimelineMessage,
  depth: number,
): TimelineMessage {
  return {
    ...message,
    depth,
  };
}

function buildDirectChildrenByParentId(messages: TimelineMessage[]) {
  const childrenByParentId = new Map<string, TimelineMessage[]>();

  for (const message of messages) {
    if (!message.parentId) {
      continue;
    }

    const children = childrenByParentId.get(message.parentId) ?? [];
    children.push(message);
    childrenByParentId.set(message.parentId, children);
  }

  return childrenByParentId;
}

function buildDescendantStatsByMessageId(
  messages: TimelineMessage[],
): Map<string, ThreadDescendantStats> {
  const messageById = new Map(messages.map((message) => [message.id, message]));
  const descendantStatsByMessageId = new Map<string, ThreadDescendantStats>(
    messages.map((message) => [
      message.id,
      {
        descendantCount: 0,
        lastReplyAt: null,
        recentParticipantsNewestFirst: [],
      },
    ]),
  );

  const orderedMessages = messages
    .map((message, index) => ({ message, index }))
    .sort((left, right) => {
      if (left.message.createdAt !== right.message.createdAt) {
        return left.message.createdAt - right.message.createdAt;
      }

      return left.index - right.index;
    });

  for (let index = orderedMessages.length - 1; index >= 0; index -= 1) {
    const message = orderedMessages[index].message;
    const participantKey = message.pubkey ?? message.id;
    const participant: TimelineThreadSummaryParticipant = {
      id: participantKey,
      author: message.author,
      avatarUrl: message.avatarUrl ?? null,
    };

    let ancestorId = message.parentId ?? null;
    let hops = 0;
    const maxHops = messages.length + 1;

    while (ancestorId && hops < maxHops) {
      const ancestorStats = descendantStatsByMessageId.get(ancestorId);
      if (!ancestorStats) {
        break;
      }

      ancestorStats.descendantCount += 1;
      ancestorStats.lastReplyAt = Math.max(
        ancestorStats.lastReplyAt ?? 0,
        message.createdAt,
      );

      if (
        ancestorStats.recentParticipantsNewestFirst.length <
          MAX_SUMMARY_PARTICIPANTS &&
        !ancestorStats.recentParticipantsNewestFirst.some(
          (existingParticipant) => existingParticipant.id === participant.id,
        )
      ) {
        ancestorStats.recentParticipantsNewestFirst.push(participant);
      }

      ancestorId = messageById.get(ancestorId)?.parentId ?? null;
      hops += 1;
    }
  }

  return descendantStatsByMessageId;
}

function buildSummaryForDirectReplies(
  messageId: string,
  descendantStatsByMessageId: Map<string, ThreadDescendantStats>,
): TimelineThreadSummary | null {
  const descendantStats = descendantStatsByMessageId.get(messageId);
  if (!descendantStats || descendantStats.descendantCount === 0) {
    return null;
  }

  return {
    threadHeadId: messageId,
    replyCount: descendantStats.descendantCount,
    lastReplyAt: descendantStats.lastReplyAt,
    participants: [...descendantStats.recentParticipantsNewestFirst].reverse(),
  };
}

function appendExpandedReplies(params: {
  entries: MainTimelineEntry[];
  parentId: string;
  depth: number;
  directChildrenByParentId: Map<string, TimelineMessage[]>;
  descendantStatsByMessageId: Map<string, ThreadDescendantStats>;
  expandedReplyIds: ReadonlySet<string>;
}) {
  const {
    entries,
    parentId,
    depth,
    directChildrenByParentId,
    descendantStatsByMessageId,
    expandedReplyIds,
  } = params;
  const directReplies = directChildrenByParentId.get(parentId) ?? [];

  for (const reply of directReplies) {
    entries.push({
      message: normalizeInlineReplyMessage(reply, depth),
      summary: buildSummaryForDirectReplies(
        reply.id,
        descendantStatsByMessageId,
      ),
    });

    if (expandedReplyIds.has(reply.id)) {
      appendExpandedReplies({
        entries,
        parentId: reply.id,
        depth: depth + 1,
        directChildrenByParentId,
        descendantStatsByMessageId,
        expandedReplyIds,
      });
    }
  }
}

function buildVisibleThreadReplies(params: {
  openThreadHeadId: string;
  directChildrenByParentId: Map<string, TimelineMessage[]>;
  descendantStatsByMessageId: Map<string, ThreadDescendantStats>;
  expandedReplyIds: ReadonlySet<string>;
}) {
  const {
    openThreadHeadId,
    directChildrenByParentId,
    descendantStatsByMessageId,
    expandedReplyIds,
  } = params;
  const entries: MainTimelineEntry[] = [];

  appendExpandedReplies({
    entries,
    parentId: openThreadHeadId,
    depth: 0,
    directChildrenByParentId,
    descendantStatsByMessageId,
    expandedReplyIds,
  });

  return entries;
}

export function buildMainTimelineEntries(
  messages: TimelineMessage[],
): MainTimelineEntry[] {
  const descendantStatsByMessageId = buildDescendantStatsByMessageId(messages);

  return messages
    .filter(
      (message) =>
        message.parentId == null || isBroadcastReply(message.tags ?? []),
    )
    .map((message) => {
      return {
        message,
        summary: buildSummaryForDirectReplies(
          message.id,
          descendantStatsByMessageId,
        ),
      };
    });
}

export function buildThreadPanelData(
  messages: TimelineMessage[],
  openThreadHeadId: string | null,
  threadReplyTargetId: string | null,
  expandedReplyIds: ReadonlySet<string>,
): ThreadPanelData {
  if (!openThreadHeadId) {
    return {
      threadHead: null,
      totalReplyCount: 0,
      visibleReplies: [],
      replyTargetMessage: null,
    };
  }

  const messageById = new Map(messages.map((message) => [message.id, message]));
  const threadHead = messageById.get(openThreadHeadId) ?? null;

  if (!threadHead) {
    return {
      threadHead: null,
      totalReplyCount: 0,
      visibleReplies: [],
      replyTargetMessage: null,
    };
  }

  const directChildrenByParentId = buildDirectChildrenByParentId(messages);
  const descendantStatsByMessageId = buildDescendantStatsByMessageId(messages);
  const normalizedThreadHead = normalizeHeadMessage(threadHead);
  const visibleReplies = buildVisibleThreadReplies({
    openThreadHeadId,
    directChildrenByParentId,
    descendantStatsByMessageId,
    expandedReplyIds,
  });

  const replyTargetInBranch =
    threadReplyTargetId === threadHead.id
      ? normalizedThreadHead
      : (messageById.get(threadReplyTargetId ?? "") ?? null);

  return {
    threadHead: normalizedThreadHead,
    totalReplyCount:
      descendantStatsByMessageId.get(openThreadHeadId)?.descendantCount ?? 0,
    visibleReplies,
    replyTargetMessage: replyTargetInBranch ?? normalizedThreadHead,
  };
}
