import type { ChannelSectionStore } from "./channelSectionsStorage";

/**
 * Whether the active community's channel list has settled enough to scope
 * section assignments. An explicit `channelsReady` flag distinguishes
 * "loaded, zero channels" from "still loading" (both present as an empty Set).
 * Without the flag, a non-empty allowlist is treated as ready for callers that
 * only pass known ids (unit tests).
 */
export function isChannelSectionsAllowlistReady(
  knownChannelIds: ReadonlySet<string> | null | undefined,
  channelsReady?: boolean,
): boolean {
  if (channelsReady === true) return true;
  if (channelsReady === false) return false;
  return Boolean(knownChannelIds && knownChannelIds.size > 0);
}

/**
 * Drop assignments whose channel id is not in the active community, and drop
 * section folders that only referenced those foreign channels.
 *
 * Used before local persist / relay publish so a stale in-memory store from a
 * previous workspace cannot write foreign channel UUIDs (or the section objects
 * that only existed to hold them) into another community's `kind:30078` blob
 * (#7207). When the allowlist is not ready, returns `store` unchanged so a
 * still-loading channel list cannot wipe every assignment.
 *
 * Intentionally empty sections (no assignments at all) are kept — they cannot
 * be distinguished from user-created empty folders on the active community.
 */
export function scopeChannelSectionsToKnownChannels(
  store: ChannelSectionStore,
  knownChannelIds: ReadonlySet<string> | null | undefined,
  channelsReady?: boolean,
): ChannelSectionStore {
  if (!isChannelSectionsAllowlistReady(knownChannelIds, channelsReady)) {
    return store;
  }
  const allow = knownChannelIds ?? new Set<string>();
  let assignmentsChanged = false;
  const assignments: Record<string, string> = {};
  for (const [channelId, sectionId] of Object.entries(store.assignments)) {
    if (allow.has(channelId)) {
      assignments[channelId] = sectionId;
    } else {
      assignmentsChanged = true;
    }
  }

  const retainedSectionIds = new Set(Object.values(assignments));
  const previouslyAssignedSectionIds = new Set(
    Object.values(store.assignments),
  );
  const sections = store.sections.filter((section) => {
    if (retainedSectionIds.has(section.id)) return true;
    // Drop sections that only held foreign-channel assignments.
    if (previouslyAssignedSectionIds.has(section.id)) return false;
    return true;
  });
  const sectionsChanged = sections.length !== store.sections.length;

  if (!assignmentsChanged && !sectionsChanged) {
    return store;
  }
  return {
    ...store,
    sections,
    assignments,
  };
}

export function swapSectionOrder(
  prev: ChannelSectionStore,
  sectionId: string,
  direction: "up" | "down",
): ChannelSectionStore | null {
  const target = prev.sections.find((s) => s.id === sectionId);
  if (!target) return null;
  const sorted = prev.sections.slice().sort((a, b) => a.order - b.order);
  const idx = sorted.findIndex((s) => s.id === sectionId);
  const neighborIdx = direction === "up" ? idx - 1 : idx + 1;
  if (neighborIdx < 0 || neighborIdx >= sorted.length) return null;
  const neighbor = sorted[neighborIdx];
  const sections = prev.sections.map((s) => {
    if (s.id === target.id) return { ...s, order: neighbor.order };
    if (s.id === neighbor.id) return { ...s, order: target.order };
    return s;
  });
  return { ...prev, sections };
}
