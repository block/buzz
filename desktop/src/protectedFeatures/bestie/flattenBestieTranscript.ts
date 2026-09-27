import type { TimelineMessage } from "@/features/messages/types";

/**
 * Flatten Bestie popover messages into a chronological top-level list.
 * Parent/reply links become regular rows so agent replies are not buried
 * under "N replies" thread stubs.
 */
export function flattenBestieTranscriptMessages(
  messages: readonly TimelineMessage[],
): TimelineMessage[] {
  return [...messages]
    .sort((left, right) => {
      if (left.createdAt !== right.createdAt) {
        return left.createdAt - right.createdAt;
      }
      return left.id.localeCompare(right.id);
    })
    .map((message) => {
      if (
        message.parentId == null &&
        message.rootId == null &&
        message.depth === 0
      ) {
        return message;
      }
      return {
        ...message,
        depth: 0,
        parentId: null,
        rootId: null,
      };
    });
}

export type BestieSessionFilterBoundary = {
  baselineMessageIds: ReadonlySet<string>;
  firstMessageCreatedAt: number;
  sessionRootId?: string;
};

/**
 * Keep messages that belong to the active Bestie session.
 * Includes threaded agent replies whose parent is in-session.
 */
export function filterBestieSessionMessages(
  messages: readonly TimelineMessage[],
  boundary: BestieSessionFilterBoundary | null,
): TimelineMessage[] {
  if (!boundary) return [];

  const inSessionIds = new Set<string>();
  const ordered = [...messages].sort((left, right) => {
    if (left.createdAt !== right.createdAt) {
      return left.createdAt - right.createdAt;
    }
    return left.id.localeCompare(right.id);
  });

  for (const message of ordered) {
    if (boundary.baselineMessageIds.has(message.id)) continue;
    const parentInSession =
      message.parentId != null && inSessionIds.has(message.parentId);
    const afterBoundary = message.createdAt >= boundary.firstMessageCreatedAt;
    if (afterBoundary || parentInSession) {
      inSessionIds.add(message.id);
    }
  }

  return ordered.filter((message) => inSessionIds.has(message.id));
}

/**
 * Continue an active Bestie session by replying in-thread to the session root.
 * First message of a session (no boundary) posts as a new channel root.
 */
export function resolveBestieSendParentEventId(
  boundary: { sessionRootId: string } | null | undefined,
): string | null {
  if (!boundary?.sessionRootId) return null;
  return boundary.sessionRootId;
}

/**
 * Thread roots whose reply subtrees belong in the active Bestie transcript.
 * Always includes the persisted session root; also includes legacy in-session
 * top-level messages from before single-thread send was fixed.
 */
export function collectBestieSessionThreadRootIds(
  boundary: BestieSessionFilterBoundary | null,
  channelMessages: readonly { id: string; parentId?: string | null; createdAt: number }[],
): string[] {
  if (!boundary) return [];
  const ids = new Set<string>();
  if (boundary.sessionRootId) ids.add(boundary.sessionRootId);
  for (const message of channelMessages) {
    if (message.parentId != null) continue;
    if (boundary.baselineMessageIds.has(message.id)) continue;
    if (message.createdAt < boundary.firstMessageCreatedAt) continue;
    ids.add(message.id);
  }
  return [...ids];
}
