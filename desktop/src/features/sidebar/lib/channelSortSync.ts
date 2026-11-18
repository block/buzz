import {
  MAX_CHANNEL_SORT_GROUPS,
  parseChannelSortPayload,
  storageKey,
  type ChannelSortMode,
} from "./channelSortPreference";
import type { Lane } from "./sidebarLaneReconciler";
import { isReg, type Tree } from "./sidebarLwwMap";

/** Non-null sort modes per group key; `null` registers are explicit resets. */
export function projectSort(tree: Tree): {
  groups: Record<string, ChannelSortMode>;
} {
  const groups: Record<string, ChannelSortMode> = {};
  for (const [key, reg] of Object.entries(tree.g ?? {})) {
    if (isReg(reg) && reg[2] !== null) groups[key] = reg[2] as ChannelSortMode;
  }
  return { groups };
}

/** Sidebar sort modes: kind 30078, d-tag `channel-sort`, one register per group key. */
export const SORT_LANE: Lane = {
  dTag: "channel-sort",
  storageKey,
  shape: { g: { "*": (v) => v === null || v === "alpha" || v === "recent" } },
  fromLegacy(json, stamp) {
    const store = parseChannelSortPayload(json);
    if (!store) return null;
    const g: Tree = Object.fromEntries(
      Object.entries(store.groups).map(([key, mode]) => [key, stamp(mode)]),
    );
    return { g };
  },
  project: projectSort,
  withinLimits: (p) =>
    Object.keys(p.groups as object).length <= MAX_CHANNEL_SORT_GROUPS,
};
