import * as React from "react";

import {
  maxLiveOrder,
  projectSections,
  SECTIONS_LANE,
} from "./channelSectionsSync";
import { useLaneSync } from "./sidebarLaneReconciler";
import { own, setRegs, type Tree } from "./sidebarLwwMap";

export type { ChannelSection } from "./channelSectionsStorage";

import type { ChannelSection } from "./channelSectionsStorage";

/**
 * One whole-list order operation against the current live list: dead IDs are
 * dropped, live IDs missing from `requested` keep their relative order at the
 * end, and every live section gets a new `order` register.
 */
function writeOrder(tree: Tree, requested: string[]) {
  const live = projectSections(tree).sections.map((s) => s.id);
  const wanted = new Set(requested.filter((id) => live.includes(id)));
  const ids = [...wanted, ...live.filter((id) => !wanted.has(id))];
  return setRegs(
    tree,
    ids.map((id, index) => [["s", id, "order"], index]),
    true,
  );
}

/**
 * Channel sections for one pubkey + relay scope, merged per field across
 * devices (see `sidebarLaneReconciler`). Deletes are `live = false`
 * tombstones; unassign is an explicit `null` register.
 */
export function useChannelSections(
  pubkey: string | undefined,
  relayUrl?: string,
): {
  sections: ChannelSection[];
  assignments: Record<string, string>;
  createSection: (name: string, icon?: string) => ChannelSection | null;
  renameSection: (sectionId: string, newName: string, icon?: string) => void;
  deleteSection: (sectionId: string) => void;
  moveSectionUp: (sectionId: string) => void;
  moveSectionDown: (sectionId: string) => void;
  reorderSections: (orderedIds: string[]) => void;
  assignChannel: (channelId: string, sectionId: string) => void;
  unassignChannel: (channelId: string) => void;
} {
  const { tree, edit } = useLaneSync(SECTIONS_LANE, pubkey, relayUrl);
  const { sections, assignments } = React.useMemo(
    () => projectSections(tree),
    [tree],
  );

  const createSection = React.useCallback(
    (name: string, icon?: string): ChannelSection | null => {
      if (!pubkey || !relayUrl) return null;
      const id = crypto.randomUUID();
      let order = 0;
      edit((prev) => {
        order = projectSections(prev).sections.length;
        return setRegs(prev, [
          [["s", id, "name"], name],
          [["s", id, "icon"], icon || null],
          [["s", id, "order"], maxLiveOrder(prev) + 1],
          [["s", id, "live"], true],
        ]);
      });
      return { id, name, ...(icon ? { icon } : {}), order };
    },
    [pubkey, relayUrl, edit],
  );

  const renameSection = React.useCallback(
    (sectionId: string, newName: string, icon?: string) =>
      edit((prev) =>
        setRegs(prev, [
          [["s", sectionId, "name"], newName],
          [["s", sectionId, "icon"], icon || null],
        ]),
      ),
    [edit],
  );

  const deleteSection = React.useCallback(
    (sectionId: string) =>
      edit((prev) => setRegs(prev, [[["s", sectionId, "live"], false]])),
    [edit],
  );

  const reorderSections = React.useCallback(
    (orderedIds: string[]) => edit((prev) => writeOrder(prev, orderedIds)),
    [edit],
  );

  const moveSection = React.useCallback(
    (sectionId: string, step: -1 | 1) =>
      edit((prev) => {
        const ids = projectSections(prev).sections.map((s) => s.id);
        const from = ids.indexOf(sectionId);
        const to = from + step;
        if (from < 0 || to < 0 || to >= ids.length) return prev;
        [ids[from], ids[to]] = [ids[to] as string, sectionId];
        return writeOrder(prev, ids);
      }),
    [edit],
  );
  const moveSectionUp = React.useCallback(
    (sectionId: string) => moveSection(sectionId, -1),
    [moveSection],
  );
  const moveSectionDown = React.useCallback(
    (sectionId: string) => moveSection(sectionId, 1),
    [moveSection],
  );

  const assignChannel = React.useCallback(
    (channelId: string, sectionId: string) =>
      edit((prev) => setRegs(prev, [[["a", channelId], sectionId]])),
    [edit],
  );

  const unassignChannel = React.useCallback(
    (channelId: string) =>
      edit((prev) =>
        prev.a && own(prev.a as Tree, channelId)
          ? setRegs(prev, [[["a", channelId], null]])
          : prev,
      ),
    [edit],
  );

  return {
    sections,
    assignments,
    createSection,
    renameSection,
    deleteSection,
    moveSectionUp,
    moveSectionDown,
    reorderSections,
    assignChannel,
    unassignChannel,
  };
}
