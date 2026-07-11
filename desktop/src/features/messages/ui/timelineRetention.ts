import type { VListHandle } from "virtua";

/**
 * Keep a wide ID-keyed neighborhood around the reader plus the visual tail.
 * The wider eviction band adds hysteresis, so small direction changes do not
 * churn mounted rows. Virtua continues to own measured sizes and spacer math.
 */
export function nextRetainedTimelineKeys(
  keys: readonly string[],
  previous: ReadonlySet<string>,
  list: VListHandle,
): ReadonlySet<string> {
  const viewportSize = Math.max(list.viewportSize, 1);
  const offset = list.scrollOffset;
  const indexAt = (target: number) =>
    list.findItemIndex(Math.min(list.scrollSize, Math.max(0, target)));
  const admissionStart = indexAt(offset - viewportSize * 8);
  const admissionEnd = indexAt(offset + viewportSize * 9);
  const evictionStart = indexAt(offset - viewportSize * 12);
  const evictionEnd = indexAt(offset + viewportSize * 13);
  const tailStart = indexAt(list.scrollSize - viewportSize * 3);
  const next = new Set<string>();

  for (let index = evictionStart; index <= evictionEnd; index += 1) {
    const key = keys[index];
    if (key && previous.has(key)) next.add(key);
  }
  for (let index = admissionStart; index <= admissionEnd; index += 1) {
    const key = keys[index];
    if (key) next.add(key);
  }
  for (let index = tailStart; index < keys.length; index += 1) {
    const key = keys[index];
    if (key) next.add(key);
  }

  return next.size === previous.size &&
    [...next].every((key) => previous.has(key))
    ? previous
    : next;
}
