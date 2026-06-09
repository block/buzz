import type { InboxFilter } from "@/features/home/lib/inbox";
import { getThreadReference } from "@/features/messages/lib/threading";
import type { RelayEvent } from "@/shared/api/types";

export function matchesInboxFilter(
  item: { categories: InboxFilter[] },
  filter: InboxFilter,
) {
  if (filter === "all") {
    return true;
  }

  return item.categories.includes(filter);
}

export function getContextMessageDepth(
  event: RelayEvent,
  eventById: ReadonlyMap<string, RelayEvent>,
): number {
  let depth = 0;
  let parentId = getThreadReference(event.tags).parentId;
  const seen = new Set<string>([event.id]);

  while (parentId && eventById.has(parentId) && !seen.has(parentId)) {
    depth += 1;
    seen.add(parentId);
    parentId = getThreadReference(eventById.get(parentId)?.tags ?? []).parentId;
  }

  return depth;
}

export function getReactionTargetId(tags: string[][]) {
  for (let index = tags.length - 1; index >= 0; index -= 1) {
    const tag = tags[index];
    if (tag?.[0] === "e" && typeof tag[1] === "string") {
      return tag[1];
    }
  }

  return null;
}
