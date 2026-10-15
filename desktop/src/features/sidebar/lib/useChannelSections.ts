import * as React from "react";

import {
  maxLiveOrder,
  projectSections,
  SECTIONS_LANE,
  siblingIds,
} from "./channelSectionsSync";
import { useLaneSync } from "./sidebarLaneReconciler";
import { own, setRegs, type Tree } from "./sidebarLwwMap";

export type { ChannelSection } from "./channelSectionsStorage";

import type { ChannelSection } from "./channelSectionsStorage";

/**
 * One whole-list order operation against one sibling group (`parentId`
 * `undefined` = top level): dead IDs are dropped, live siblings missing from
 * `requested` keep their relative order at the end, and every section in the
 * group gets a new `order` register. Other groups are left alone — `order`
 * only ever competes between siblings.
 */
function writeOrder(tree: Tree, requested: string[], parentId?: string) {
  const live = siblingIds(projectSections(tree).sections, parentId);
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
  createSection: (
    name: string,
    icon?: string,
    parentId?: string,
  ) => ChannelSection | null;
  renameSection: (sectionId: string, newName: string, icon?: string) => void;
  deleteSection: (sectionId: string) => void;
  setSectionParent: (sectionId: string, parentId: string | null) => void;
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
    (name: string, icon?: string, parentId?: string): ChannelSection | null => {
      if (!pubkey || !relayUrl) return null;
      const id = crypto.randomUUID();
      let created: ChannelSection = {
        id,
        name,
        ...(icon ? { icon } : {}),
        order: 0,
      };
      edit((prev) => {
        const { sections } = projectSections(prev);
        // Only a top-level section can take children; any other target
        // (missing, deleted, itself nested) creates at the top level.
        const nestUnder = sections.find(
          (section) => section.id === parentId && !section.parentId,
        )?.id;
        created = {
          id,
          name,
          ...(icon ? { icon } : {}),
          order: siblingIds(sections, nestUnder).length,
          ...(nestUnder ? { parentId: nestUnder } : {}),
        };
        return setRegs(prev, [
          [["s", id, "name"], name],
          [["s", id, "icon"], icon || null],
          [["s", id, "order"], maxLiveOrder(prev) + 1],
          [["s", id, "live"], true],
          [["s", id, "parent"], nestUnder ?? null],
        ]);
      });
      return created;
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

  /**
   * Nests `sectionId` under `parentId`, or lifts it back to the top level with
   * `null`. Refused when it would open a third level — the target is itself
   * nested, or the section already has children — or point a section at itself.
   * A moved section lands last among its new siblings.
   */
  const setSectionParent = React.useCallback(
    (sectionId: string, parentId: string | null) =>
      edit((prev) => {
        const { sections } = projectSections(prev);
        const section = sections.find((entry) => entry.id === sectionId);
        if (!section) return prev;
        const nextParent =
          parentId === null
            ? null
            : sections.find(
                (entry) =>
                  entry.id === parentId &&
                  entry.id !== sectionId &&
                  !entry.parentId,
              )?.id;
        if (nextParent === undefined) return prev;
        if (nextParent === (section.parentId ?? null)) return prev;
        if (
          nextParent !== null &&
          sections.some((entry) => entry.parentId === sectionId)
        ) {
          return prev;
        }
        return setRegs(prev, [
          [["s", sectionId, "parent"], nextParent],
          [["s", sectionId, "order"], maxLiveOrder(prev) + 1],
        ]);
      }),
    [edit],
  );

  /** Reorders the top-level sections; subsections keep their own order. */
  const reorderSections = React.useCallback(
    (orderedIds: string[]) => edit((prev) => writeOrder(prev, orderedIds)),
    [edit],
  );

  const moveSection = React.useCallback(
    (sectionId: string, step: -1 | 1) =>
      edit((prev) => {
        const { sections } = projectSections(prev);
        const parentId = sections.find(
          (section) => section.id === sectionId,
        )?.parentId;
        const ids = siblingIds(sections, parentId);
        const from = ids.indexOf(sectionId);
        const to = from + step;
        if (from < 0 || to < 0 || to >= ids.length) return prev;
        [ids[from], ids[to]] = [ids[to] as string, sectionId];
        return writeOrder(prev, ids, parentId);
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
    setSectionParent,
    moveSectionUp,
    moveSectionDown,
    reorderSections,
    assignChannel,
    unassignChannel,
  };
}
