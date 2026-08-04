import type { ChannelSectionStore } from "./channelSectionsStorage";

/** Sortable id for the built-in uncategorized Channels block in block order. */
export const CHANNELS_BLOCK_ID = "__channels__";

/**
 * Resolves where the uncategorized Channels block sits among custom categories.
 * Missing / non-integer / out-of-range values map to "after all categories"
 * (the historical layout). Non-integers are never truncated into a different
 * position — that would silently rewrite remote state.
 */
export function resolveChannelsBlockIndex(
  store: Pick<ChannelSectionStore, "sections" | "channelsBlockIndex">,
): number {
  const sectionCount = store.sections.length;
  const raw = store.channelsBlockIndex;
  if (typeof raw !== "number" || !Number.isInteger(raw)) return sectionCount;
  return Math.max(0, Math.min(sectionCount, raw));
}

/**
 * Optional field normalization for parse: only persist an integer in range.
 * Non-integers (e.g. 1.5), non-numbers, and out-of-range → undefined
 * (legacy layout). Do not truncate floats — that silently changes remote state.
 */
export function normalizeChannelsBlockIndex(
  value: unknown,
  sectionCount: number,
): number | undefined {
  if (typeof value !== "number" || !Number.isInteger(value)) return undefined;
  if (value < 0 || value > sectionCount) return undefined;
  return value;
}

/**
 * Display / sortable order of movable sidebar blocks: custom category ids plus
 * the Channels sentinel. Starred / Forums / DMs stay outside this list.
 */
export function getSidebarBlockOrder(
  store: Pick<ChannelSectionStore, "sections" | "channelsBlockIndex">,
): string[] {
  const sorted = store.sections.slice().sort((a, b) => a.order - b.order);
  const ids = sorted.map((section) => section.id);
  const index = resolveChannelsBlockIndex(store);
  return [...ids.slice(0, index), CHANNELS_BLOCK_ID, ...ids.slice(index)];
}

/**
 * Apply a full block order (section ids + {@link CHANNELS_BLOCK_ID}).
 * Unknown ids are ignored; missing live sections append at the end (before
 * Channels if Channels was not listed, matching default layout).
 */
export function applySidebarBlockOrder(
  prev: ChannelSectionStore,
  orderedBlockIds: readonly string[],
): ChannelSectionStore {
  const liveIds = new Set(prev.sections.map((section) => section.id));
  const seen = new Set<string>();
  const orderedSectionIds: string[] = [];
  let channelsIndex = -1;

  for (const id of orderedBlockIds) {
    if (id === CHANNELS_BLOCK_ID) {
      if (channelsIndex === -1) channelsIndex = orderedSectionIds.length;
      continue;
    }
    if (!liveIds.has(id) || seen.has(id)) continue;
    seen.add(id);
    orderedSectionIds.push(id);
  }
  for (const section of prev.sections
    .slice()
    .sort((a, b) => a.order - b.order)) {
    if (!seen.has(section.id)) orderedSectionIds.push(section.id);
  }
  if (channelsIndex === -1) channelsIndex = orderedSectionIds.length;
  channelsIndex = Math.max(
    0,
    Math.min(orderedSectionIds.length, channelsIndex),
  );

  const orderById = new Map(
    orderedSectionIds.map((id, order) => [id, order] as const),
  );
  const sections = prev.sections.map((section) => ({
    ...section,
    order: orderById.get(section.id) ?? section.order,
  }));

  return {
    ...prev,
    sections,
    channelsBlockIndex: channelsIndex,
  };
}

export function swapBlockOrder(
  prev: ChannelSectionStore,
  blockId: string,
  direction: "up" | "down",
): ChannelSectionStore | null {
  const order = getSidebarBlockOrder(prev);
  const idx = order.indexOf(blockId);
  if (idx === -1) return null;
  const neighborIdx = direction === "up" ? idx - 1 : idx + 1;
  if (neighborIdx < 0 || neighborIdx >= order.length) return null;
  const next = order.slice();
  const current = next[idx];
  const neighbor = next[neighborIdx];
  if (current === undefined || neighbor === undefined) return null;
  next[idx] = neighbor;
  next[neighborIdx] = current;
  return applySidebarBlockOrder(prev, next);
}

/**
 * Remove a category and adjust {@link channelsBlockIndex} so the Channels
 * sentinel keeps its relative place among remaining blocks.
 *
 * Example: `[A, Channels, B]` (`channelsBlockIndex=1`) deleting A →
 * `[Channels, B]` (`channelsBlockIndex=0`), not `[B, Channels]`.
 */
export function removeSectionFromStore(
  prev: ChannelSectionStore,
  sectionId: string,
): ChannelSectionStore {
  const order = getSidebarBlockOrder(prev).filter((id) => id !== sectionId);
  const sections = prev.sections.filter((section) => section.id !== sectionId);
  const assignments = { ...prev.assignments };
  for (const channelId of Object.keys(assignments)) {
    if (assignments[channelId] === sectionId) {
      delete assignments[channelId];
    }
  }
  return applySidebarBlockOrder({ ...prev, sections, assignments }, order);
}

/**
 * Append a new category at the end of the movable lane without shifting
 * Channels relative to existing categories (channelsBlockIndex unchanged
 * when it sits among existing sections; when omitted, resolve stays "after
 * all categories" including the new one).
 */
export function appendSectionToStore(
  prev: ChannelSectionStore,
  section: ChannelSectionStore["sections"][number],
): ChannelSectionStore {
  const maxOrder =
    prev.sections.length > 0
      ? Math.max(...prev.sections.map((s) => s.order))
      : -1;
  const nextSection = { ...section, order: maxOrder + 1 };
  return {
    ...prev,
    sections: [...prev.sections, nextSection],
  };
}

/** @deprecated Prefer {@link swapBlockOrder}; kept for category-only swaps. */
export function swapSectionOrder(
  prev: ChannelSectionStore,
  sectionId: string,
  direction: "up" | "down",
): ChannelSectionStore | null {
  return swapBlockOrder(prev, sectionId, direction);
}
