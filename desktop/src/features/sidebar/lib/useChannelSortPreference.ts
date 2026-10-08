import * as React from "react";

import {
  DEFAULT_SORT_MODE,
  type ChannelSortGroupKey,
  type ChannelSortMode,
} from "./channelSortPreference";
import { SORT_LANE } from "./channelSortSync";
import { useLaneSync } from "./sidebarLaneReconciler";
import { isReg, own, setRegs } from "./sidebarLwwMap";

/**
 * Persistent per-group sidebar sort preferences, scoped by pubkey + relay.
 * Each grouping (starred, channels, forums, dms, and each custom section)
 * carries its own Recent/A–Z mode; unset groups default to A–Z. Each group
 * is its own register, so devices merge per group (see
 * `sidebarLaneReconciler`) rather than overwriting the whole blob.
 */
export function useChannelSortPreference(
  pubkey: string | undefined,
  relayUrl?: string,
): {
  sortModeFor: (group: ChannelSortGroupKey) => ChannelSortMode;
  setSortModeFor: (group: ChannelSortGroupKey, mode: ChannelSortMode) => void;
} {
  const { tree, edit } = useLaneSync(SORT_LANE, pubkey, relayUrl);

  const sortModeFor = React.useCallback(
    (group: ChannelSortGroupKey) => {
      const reg = tree.g && !isReg(tree.g) ? own(tree.g, group) : undefined;
      return isReg(reg) && reg[2] !== null
        ? (reg[2] as ChannelSortMode)
        : DEFAULT_SORT_MODE;
    },
    [tree],
  );

  const setSortModeFor = React.useCallback(
    (group: ChannelSortGroupKey, mode: ChannelSortMode) =>
      edit((prev) => setRegs(prev, [[["g", group], mode]])),
    [edit],
  );

  return { sortModeFor, setSortModeFor };
}
