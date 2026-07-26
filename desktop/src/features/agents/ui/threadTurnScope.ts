/**
 * Scope a transcript to one thread's turns.
 *
 * Observer frames carry `turnId` but no thread reference, so a thread's turns
 * have to be identified indirectly: a user row carries the `messageId` that
 * triggered its turn, and we know which message ids belong to this thread.
 *
 * The rule is **exclude only what is provably foreign**, not "include only what
 * is provably ours". That distinction matters a lot in practice: a whitelist
 * drops every turn it cannot attribute — including the turn that is running
 * right now, whose prompt row may not exist yet (and on Claude Code's
 * cancel+merge path may never exist, because no `steer:` frame is written). The
 * result is an agent that appears to do nothing while it works. Excluding only
 * attributable-elsewhere turns keeps threads separate *and* keeps live activity
 * visible.
 *
 * Items with no `turnId` are session-level (connection, mode, command lists)
 * rather than thread content, so they are kept.
 */

export type ThreadScopeItem = {
  id: string;
  type?: string;
  role?: string;
  messageId?: string | null;
  turnId?: string | null;
};

export type TurnAttribution = "own" | "foreign" | "unattributed";

/**
 * Classify each turn by the user messages seen inside it.
 *
 * - `own` — carries at least one of this thread's messages
 * - `foreign` — carries user messages, none of them this thread's
 * - `unattributed` — no user message id seen yet (e.g. the in-flight turn)
 */
export function classifyTurns(
  items: readonly ThreadScopeItem[],
  threadMessageIds: ReadonlySet<string>,
): Map<string, TurnAttribution> {
  const seenByTurn = new Map<string, { own: boolean; any: boolean }>();

  for (const item of items) {
    if (!item.turnId) {
      continue;
    }
    const entry = seenByTurn.get(item.turnId) ?? { own: false, any: false };
    if (item.type === "message" && item.role === "user" && item.messageId) {
      entry.any = true;
      if (threadMessageIds.has(item.messageId)) {
        entry.own = true;
      }
    }
    seenByTurn.set(item.turnId, entry);
  }

  const result = new Map<string, TurnAttribution>();
  for (const [turnId, entry] of seenByTurn) {
    result.set(
      turnId,
      entry.own ? "own" : entry.any ? "foreign" : "unattributed",
    );
  }
  return result;
}

export function scopeItemsToThread<T extends ThreadScopeItem>(
  items: readonly T[],
  threadMessageIds: ReadonlySet<string> | undefined,
  isInjectedId: (id: string) => boolean,
): T[] {
  if (!threadMessageIds || threadMessageIds.size === 0) {
    return items as T[];
  }
  const attribution = classifyTurns(items, threadMessageIds);

  return items.filter((item) => {
    // Injected rows are built from one thread's messages and carry no turnId.
    if (isInjectedId(item.id)) {
      return true;
    }
    if (!item.turnId) {
      return true;
    }
    return attribution.get(item.turnId) !== "foreign";
  });
}
