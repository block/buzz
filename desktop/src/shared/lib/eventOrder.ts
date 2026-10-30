type OrderedEvent = { created_at: number; id: string };

/** Relay order: newest timestamp first, then ascending id within a second. */
export function compareRelayOrder(left: OrderedEvent, right: OrderedEvent) {
  return left.created_at !== right.created_at
    ? right.created_at - left.created_at
    : left.id < right.id
      ? -1
      : left.id > right.id
        ? 1
        : 0;
}

/**
 * Timeline order, oldest first: the exact reverse of relay order, so a larger
 * id sorts first within a second. Every timeline sort uses this one order so
 * the live cache and the channel window never swap same-second rows, and an
 * older page fetched from a mid-second cursor lands contiguously above the
 * rows already shown.
 */
export function compareChronologicalOrder(
  left: OrderedEvent,
  right: OrderedEvent,
) {
  return compareRelayOrder(right, left);
}
