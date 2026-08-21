import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";

/**
 * True when a later entry sits at exactly the same depth as `entryIndex`
 * before the branch closes out to a shallower depth — i.e. the entry still
 * has a visible sibling below it, so its parent's guide line must continue.
 */
function hasLaterVisibleSibling(
  entries: readonly MainTimelineEntry[],
  entryIndex: number,
): boolean {
  const depth = entries[entryIndex]?.message.depth;
  if (depth == null) {
    return false;
  }

  for (let index = entryIndex + 1; index < entries.length; index += 1) {
    const nextDepth = entries[index].message.depth;
    if (nextDepth <= depth) {
      return nextDepth === depth;
    }
  }

  return false;
}

/**
 * Depths whose thread-depth guide line must keep running past the entry at
 * `index`, because the ancestor at that depth still has a visible child
 * sibling further down the flattened thread.
 */
export function getActiveContinuationDepths({
  ancestors,
  entries,
  index,
  message,
}: {
  ancestors: readonly { index: number; message: TimelineMessage }[];
  entries: readonly MainTimelineEntry[];
  index: number;
  message: TimelineMessage;
}): number[] {
  const depths: number[] = [];

  for (const ancestor of ancestors) {
    if (ancestor.message.depth === 0) {
      continue;
    }

    const childDepth = ancestor.message.depth + 1;
    const pathChild =
      message.depth === childDepth
        ? { index, message }
        : ancestors.find((candidate) => candidate.message.depth === childDepth);

    if (pathChild && hasLaterVisibleSibling(entries, pathChild.index)) {
      depths.push(ancestor.message.depth);
    }
  }

  return depths;
}
