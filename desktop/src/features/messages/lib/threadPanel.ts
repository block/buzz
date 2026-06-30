import type { TimelineMessage } from "@/features/messages/types";
import { isBroadcastReply } from "@/features/messages/lib/threading";
import { KIND_HUDDLE_STARTED } from "@/shared/constants/kinds";

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

export type ThreadDescendantStats = {
  descendantCount: number;
  unreadDescendantCount: number;
  lastReplyAt: number | null;
  recentParticipantsNewestFirst: TimelineThreadSummaryParticipant[];
};

export type ThreadPanelIndex = {
  directChildrenByParentId: Map<string, TimelineMessage[]>;
  descendantStatsByMessageId: Map<string, ThreadDescendantStats>;
  messageById: Map<string, TimelineMessage>;
};

const MAX_SUMMARY_PARTICIPANTS = 3;

type SummaryParticipantCandidate = {
  index: number;
  participant: TimelineThreadSummaryParticipant;
  timestamp: number;
};

function normalizeHeadMessage(message: TimelineMessage): TimelineMessage {
  return {
    ...message,
    depth: 0,
  };
}

// Thread rows feed `MessageRow` a depth-normalized copy of each reply. Building
// that copy fresh (`{ ...message, depth }`) on every render hands `MessageRow` a
// new object identity every time `timelineMessages` churns (typing/presence),
// even when the reply and its depth are byte-identical — which defeats the
// row/markdown memo and forces a ~1.4ms/row re-parse on threads where the main
// timeline (which passes the raw stable ref) stays cheap.
//
// Mirror the main list's per-id context memoization (`videoReviewContextById`):
// cache the normalized object keyed on the source reply identity + depth, so an
// unrelated channel churn that leaves a reply (and its tree position) intact
// reuses the exact same object reference and the memo hits.
//
// Keyed on the source `reply` reference via a WeakMap: a new `timelineMessages`
// set produces new reply objects (genuine recompute), and stale entries are
// collected automatically when the old message set is dropped.
const normalizedInlineReplyCache = new WeakMap<
  TimelineMessage,
  Map<number, TimelineMessage>
>();

function normalizeInlineReplyMessage(
  message: TimelineMessage,
  depth: number,
): TimelineMessage {
  let byDepth = normalizedInlineReplyCache.get(message);
  if (!byDepth) {
    byDepth = new Map<number, TimelineMessage>();
    normalizedInlineReplyCache.set(message, byDepth);
  }

  const cached = byDepth.get(depth);
  if (cached) {
    return cached;
  }

  const normalized: TimelineMessage = {
    ...message,
    depth,
  };
  byDepth.set(depth, normalized);
  return normalized;
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

export function buildDescendantStatsByMessageId(
  messages: TimelineMessage[],
  unreadReplyIds: ReadonlySet<string> = new Set(),
  messageById: Map<string, TimelineMessage> = new Map(
    messages.map((message) => [message.id, message]),
  ),
): Map<string, ThreadDescendantStats> {
  const descendantStatsByMessageId = new Map<string, ThreadDescendantStats>(
    messages.map((message) => [
      message.id,
      {
        descendantCount: 0,
        unreadDescendantCount: 0,
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
    const isUnread = unreadReplyIds.has(message.id);

    while (ancestorId && hops < maxHops) {
      const ancestorStats = descendantStatsByMessageId.get(ancestorId);
      if (!ancestorStats) {
        break;
      }

      ancestorStats.descendantCount += 1;
      if (isUnread) {
        ancestorStats.unreadDescendantCount += 1;
      }
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

export function buildThreadPanelIndex(
  messages: TimelineMessage[],
  unreadReplyIds: ReadonlySet<string> = new Set(),
): ThreadPanelIndex {
  const messageById = new Map(messages.map((message) => [message.id, message]));

  return {
    directChildrenByParentId: buildDirectChildrenByParentId(messages),
    descendantStatsByMessageId: buildDescendantStatsByMessageId(
      messages,
      unreadReplyIds,
      messageById,
    ),
    messageById,
  };
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

function participantFromMessage(
  message: TimelineMessage,
): TimelineThreadSummaryParticipant {
  return {
    id: message.pubkey ?? message.id,
    author: message.author,
    avatarUrl: message.avatarUrl ?? null,
  };
}

export function buildThreadSummaryFromVisibleEntries(
  threadHeadId: string,
  entries: readonly MainTimelineEntry[],
): TimelineThreadSummary | null {
  let replyCount = 0;
  let lastReplyAt: number | null = null;
  const participantCandidates: SummaryParticipantCandidate[] = [];

  const addParticipantCandidate = (
    participant: TimelineThreadSummaryParticipant,
    timestamp: number,
  ) => {
    participantCandidates.push({
      index: participantCandidates.length,
      participant,
      timestamp,
    });
  };

  for (const entry of entries) {
    replyCount += 1;
    lastReplyAt = Math.max(lastReplyAt ?? 0, entry.message.createdAt);
    addParticipantCandidate(
      participantFromMessage(entry.message),
      entry.message.createdAt,
    );

    if (entry.summary) {
      replyCount += entry.summary.replyCount;
      if (entry.summary.lastReplyAt != null) {
        lastReplyAt = Math.max(lastReplyAt ?? 0, entry.summary.lastReplyAt);
      }

      const summaryTimestamp =
        entry.summary.lastReplyAt ?? entry.message.createdAt;
      for (const participant of entry.summary.participants) {
        addParticipantCandidate(participant, summaryTimestamp);
      }
    }
  }

  if (replyCount === 0) {
    return null;
  }

  const recentParticipantsNewestFirst: TimelineThreadSummaryParticipant[] = [];
  for (const candidate of [...participantCandidates].sort((left, right) => {
    if (left.timestamp !== right.timestamp) {
      return right.timestamp - left.timestamp;
    }

    return right.index - left.index;
  })) {
    if (
      recentParticipantsNewestFirst.some(
        (participant) => participant.id === candidate.participant.id,
      )
    ) {
      continue;
    }

    recentParticipantsNewestFirst.push(candidate.participant);
    if (recentParticipantsNewestFirst.length >= MAX_SUMMARY_PARTICIPANTS) {
      break;
    }
  }

  return {
    threadHeadId,
    replyCount,
    lastReplyAt,
    participants: recentParticipantsNewestFirst.reverse(),
  };
}

export function hasNestedThreadBranches(entries: readonly MainTimelineEntry[]) {
  return entries.some(
    (entry) => entry.message.depth > 1 || entry.summary !== null,
  );
}

function isDescendantOfMessage(
  message: TimelineMessage,
  ancestorId: string,
  messageById: Map<string, TimelineMessage>,
) {
  if (message.id === ancestorId) {
    return false;
  }

  if (message.rootId === ancestorId || message.parentId === ancestorId) {
    return true;
  }

  let parentId = message.parentId ?? null;
  let hops = 0;
  const maxHops = messageById.size + 1;
  while (parentId && hops < maxHops) {
    if (parentId === ancestorId) {
      return true;
    }

    parentId = messageById.get(parentId)?.parentId ?? null;
    hops += 1;
  }

  return false;
}

function buildVisibleThreadReplies(params: {
  openThreadHeadId: string;
  messageById: Map<string, TimelineMessage>;
}) {
  const { openThreadHeadId, messageById } = params;

  return [...messageById.values()]
    .map((message, index) => ({ index, message }))
    .filter(({ message }) =>
      isDescendantOfMessage(message, openThreadHeadId, messageById),
    )
    .sort((left, right) => {
      if (left.message.createdAt !== right.message.createdAt) {
        return left.message.createdAt - right.message.createdAt;
      }

      return left.index - right.index;
    })
    .map(
      ({ message }): MainTimelineEntry => ({
        message: normalizeInlineReplyMessage(message, 1),
        summary: null,
      }),
    );
}

export function buildMainTimelineEntries(
  messages: TimelineMessage[],
  unreadReplyIds: ReadonlySet<string> = new Set(),
): MainTimelineEntry[] {
  const { descendantStatsByMessageId } = buildThreadPanelIndex(
    messages,
    unreadReplyIds,
  );

  return messages
    .filter(
      (message) =>
        message.parentId == null || isBroadcastReply(message.tags ?? []),
    )
    .map((message) => {
      return {
        message,
        summary:
          message.kind === KIND_HUDDLE_STARTED
            ? null
            : buildSummaryForDirectReplies(
                message.id,
                descendantStatsByMessageId,
              ),
      };
    });
}

/**
 * Whether the unread "New" divider should render above the entry at `index`.
 * The divider marks a read/unread boundary, so it only makes sense when there
 * is a rendered message above the first unread. When the first unread is the
 * first rendered top-level entry (index 0) — the fresh/never-read channel case
 * — there is nothing above it to separate from, so the divider is suppressed.
 */
export function shouldRenderUnreadDivider(
  index: number,
  messageId: string,
  firstUnreadMessageId: string | null,
): boolean {
  return index > 0 && messageId === firstUnreadMessageId;
}

export function buildThreadPanelDataFromIndex(
  index: ThreadPanelIndex,
  openThreadHeadId: string | null,
  threadReplyTargetId: string | null,
  _expandedReplyIds: ReadonlySet<string>,
): ThreadPanelData {
  if (!openThreadHeadId) {
    return {
      threadHead: null,
      totalReplyCount: 0,
      visibleReplies: [],
      replyTargetMessage: null,
    };
  }

  const { descendantStatsByMessageId, messageById } = index;
  const threadHead = messageById.get(openThreadHeadId) ?? null;

  if (!threadHead) {
    return {
      threadHead: null,
      totalReplyCount: 0,
      visibleReplies: [],
      replyTargetMessage: null,
    };
  }

  const normalizedThreadHead = normalizeHeadMessage(threadHead);
  const visibleReplies = buildVisibleThreadReplies({
    openThreadHeadId,
    messageById,
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

export function buildThreadPanelData(
  messages: TimelineMessage[],
  openThreadHeadId: string | null,
  threadReplyTargetId: string | null,
  expandedReplyIds: ReadonlySet<string>,
): ThreadPanelData {
  return buildThreadPanelDataFromIndex(
    buildThreadPanelIndex(messages),
    openThreadHeadId,
    threadReplyTargetId,
    expandedReplyIds,
  );
}
