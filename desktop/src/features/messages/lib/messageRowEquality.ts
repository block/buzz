import type {
  TimelineMessage,
  TimelineReaction,
} from "@/features/messages/types";

/**
 * Value-equality helpers for `MessageRow`'s memo comparator.
 *
 * Several row props are rebuilt with fresh identities on every ingest even
 * when their values are unchanged: `message.tags` (every relay/IPC refetch
 * deserializes new event objects), `message.reactions` (rebuilt per
 * `formatTimelineMessages` run), and the thread panel's per-row layout
 * arrays. Comparing them by identity made every row re-render on every
 * streamed event — in a long open thread with agents streaming, that
 * re-reconciled every row's action bar/tooltip/provider subtree several
 * times per second and saturated the main thread (see
 * typing-latency.perf.ts, scenario "thread68+"). These value comparisons
 * are O(a few tiny arrays) per row — far cheaper than one spurious row
 * render.
 */

export function tagsEqual(
  a: TimelineMessage["tags"],
  b: TimelineMessage["tags"],
): boolean {
  if (a === b) return true;
  if (!a || !b || a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) {
    const tagA = a[i];
    const tagB = b[i];
    if (tagA.length !== tagB.length) return false;
    for (let j = 0; j < tagA.length; j += 1) {
      if (tagA[j] !== tagB[j]) return false;
    }
  }
  return true;
}

export function reactionsEqual(
  a: TimelineReaction[] | undefined,
  b: TimelineReaction[] | undefined,
): boolean {
  if (a === b) return true;
  if (!a || !b || a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) {
    const left = a[i];
    const right = b[i];
    if (
      left.emoji !== right.emoji ||
      left.emojiUrl !== right.emojiUrl ||
      left.count !== right.count ||
      left.reactedByCurrentUser !== right.reactedByCurrentUser ||
      left.users.length !== right.users.length
    ) {
      return false;
    }
    for (let j = 0; j < left.users.length; j += 1) {
      const userA = left.users[j];
      const userB = right.users[j];
      if (
        userA.pubkey !== userB.pubkey ||
        userA.displayName !== userB.displayName ||
        userA.avatarUrl !== userB.avatarUrl
      ) {
        return false;
      }
    }
  }
  return true;
}

export function requestStatusEqual(
  a: TimelineMessage["requestStatus"],
  b: TimelineMessage["requestStatus"],
): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  if (a.kind !== b.kind) return false;
  if (a.kind === "invalid" || a.kind === "unsupported") {
    // Narrowing `a` does not narrow `b`; the kind check above guarantees they
    // match, and both carry `reason` in these arms.
    return a.reason === (b as typeof a).reason;
  }
  const other = b as typeof a;
  return (
    a.outcome.kind === other.outcome.kind &&
    // `state` is absent on unanswered/disputed outcomes; reading it as an
    // optional field covers all four shapes without a second switch.
    (a.outcome as { state?: string }).state ===
      (other.outcome as { state?: string }).state &&
    a.latestObservation === other.latestObservation &&
    a.reason === other.reason &&
    a.resolved === other.resolved &&
    // Compared by value, not identity: the warning array is rebuilt fresh on
    // every format pass, so an identity check would defeat the memo for every
    // row carrying a request status.
    a.warnings.length === other.warnings.length &&
    a.warnings.every((warning, i) => warning === other.warnings[i])
  );
}

export function numberArrayEqual(
  a: readonly number[] | undefined,
  b: readonly number[] | undefined,
): boolean {
  if (a === b) return true;
  if (!a || !b || a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

/** Thread depth-guide actions: value-equal when every action matches on its
 * scalar fields and refers to the same ancestor message (by id). Structural
 * type mirrors `ThreadDepthGuideAction` (MessageRow.tsx) — importing it here
 * would create an import cycle with MessageRow. */
export function depthGuideActionsEqual(
  a:
    | ReadonlyArray<{
        active?: boolean;
        depth: number;
        label: string;
        message: TimelineMessage;
      }>
    | undefined,
  b:
    | ReadonlyArray<{
        active?: boolean;
        depth: number;
        label: string;
        message: TimelineMessage;
      }>
    | undefined,
): boolean {
  if (a === b) return true;
  if (!a || !b || a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) {
    if (
      a[i].active !== b[i].active ||
      a[i].depth !== b[i].depth ||
      a[i].label !== b[i].label ||
      a[i].message.id !== b[i].message.id
    ) {
      return false;
    }
  }
  return true;
}
