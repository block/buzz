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

// A.3.1: the channel-wide descendant walk is O(N x avg-depth) and depends ONLY
// on the timeline message set. Both render paths (main timeline + thread panel)
// need it, so it is exported to be computed once per `timelineMessages` change
// and shared, instead of re-walking the whole channel on every thread-open /
// expand. Memoize this on `messages` identity at the call site.
export function buildDescendantStatsByMessageId(
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
  precomputedDescendantStatsByMessageId?: Map<string, ThreadDescendantStats>,
): MainTimelineEntry[] {
  const descendantStatsByMessageId =
    precomputedDescendantStatsByMessageId ??
    buildDescendantStatsByMessageId(messages);

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
  precomputedDescendantStatsByMessageId?: Map<string, ThreadDescendantStats>,
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

  // ====================================================================
  // LAYER 1 INSTRUMENTATION (dev-only, throwaway — RIP OUT THE WHOLE BLOCK)
  // Measures the synchronous blocking compute in the thread-open commit
  // path. tho's tell (whole-app freeze -> fully-loaded pane) says the cost
  // is synchronous main-thread work, so we bracket each seam with
  // performance.now() and emit one readable line per thread-open. The
  // descendantStats seam self-diagnoses whether A.3.1 held (cache hit) or
  // the memo flipped (RECOMPUTED). Gated on import.meta.env?.DEV: Vite
  // strips it in prod (zero cost), and it is undefined under the node test
  // loader so the microbench never pays for it.
  const __dev = import.meta.env?.DEV === true;
  const __t0 = __dev ? performance.now() : 0;

  const directChildrenByParentId = buildDirectChildrenByParentId(messages);
  const __tDirectChildren = __dev ? performance.now() : 0;

  const __statsWasShared = precomputedDescendantStatsByMessageId != null;
  const descendantStatsByMessageId =
    precomputedDescendantStatsByMessageId ??
    buildDescendantStatsByMessageId(messages);
  const __tDescendantStats = __dev ? performance.now() : 0;

  const normalizedThreadHead = normalizeHeadMessage(threadHead);
  const visibleReplies = buildVisibleThreadReplies({
    openThreadHeadId,
    directChildrenByParentId,
    descendantStatsByMessageId,
    expandedReplyIds,
  });

  if (__dev) {
    const __tEnd = performance.now();
    const ms = (a: number, b: number) => (b - a).toFixed(1);
    const statsLabel = __statsWasShared
      ? "cache hit"
      : "RECOMPUTED — A.3.1 cache missed!";
    console.log(
      `[timeline] thread-open ${openThreadHeadId.slice(0, 8)}: ` +
        `total=${ms(__t0, __tEnd)}ms | ` +
        `directChildren=${ms(__t0, __tDirectChildren)}ms | ` +
        `descendantStats=${ms(__tDirectChildren, __tDescendantStats)}ms (${statsLabel}) | ` +
        `visibleReplies=${ms(__tDescendantStats, __tEnd)}ms`,
    );
  }
  // ============ END LAYER 1 INSTRUMENTATION (RIP OUT ABOVE) ============

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
