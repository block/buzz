import {
  MAX_CHANNEL_SECTION_ASSIGNMENTS,
  MAX_CHANNEL_SECTIONS,
  parseChannelSectionPayload,
  storageKey,
  type ChannelSection,
} from "./channelSectionsStorage";
import type { Lane } from "./sidebarLaneReconciler";
import { isReg, type Reg, type Tree } from "./sidebarLwwMap";

const isString = (value: unknown) => typeof value === "string";

type SectionNode = { name?: Reg; icon?: Reg; order?: Reg; live?: Reg };

export type SectionsView = {
  sections: ChannelSection[];
  assignments: Record<string, string>;
};

const val = (reg: unknown) => (isReg(reg) ? reg[2] : undefined);

/** Live sections by `(order, id)`, with their canonical `order`. */
function rankedSections(tree: Tree) {
  const nodes = (tree.s ?? {}) as Record<string, SectionNode>;
  return Object.entries(nodes)
    .filter(([, n]) => val(n.live) === true && isString(val(n.name)))
    .map(([id, n]) => ({
      id,
      name: val(n.name) as string,
      icon: val(n.icon),
      order: Number(val(n.order) ?? 0),
    }))
    .sort(
      (a, b) => a.order - b.order || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0),
    );
}

/** Highest canonical `order` among live sections, or -1 when there are none. */
export const maxLiveOrder = (tree: Tree) =>
  Math.max(-1, ...rankedSections(tree).map((s) => s.order));

/** Live sections, densely renumbered. */
function liveSections(tree: Tree): ChannelSection[] {
  return rankedSections(tree).map(({ icon, ...s }, order) => ({
    ...s,
    ...(isString(icon) && icon ? { icon: icon as string } : {}),
    order,
  }));
}

/** The sections and assignments older clients (and this UI) read. */
export function projectSections(tree: Tree): SectionsView {
  const sections = liveSections(tree);
  const live = new Set(sections.map((s) => s.id));
  const assignments: Record<string, string> = {};
  for (const [channelId, reg] of Object.entries(tree.a ?? {})) {
    const target = val(reg);
    if (isString(target) && live.has(target as string))
      assignments[channelId] = target as string;
  }
  return { sections, assignments };
}

/** Channel sections: kind 30078, d-tag `channel-sections`, per-field registers. */
export const SECTIONS_LANE: Lane = {
  dTag: "channel-sections",
  storageKey,
  legacyStorageKey: (pubkey) => storageKey(pubkey),
  shape: {
    s: {
      "*": {
        name: isString,
        icon: (v) => v === null || isString(v),
        order: Number.isSafeInteger,
        live: (v) => typeof v === "boolean",
      },
    },
    a: { "*": (v) => v === null || isString(v) },
  },
  fromLegacy(json, stamp) {
    const store = parseChannelSectionPayload(json);
    if (!store) return null;
    const s: Tree = Object.fromEntries(
      store.sections.map(({ id, name, icon, order }) => [
        id,
        {
          name: stamp(name),
          icon: stamp(icon ?? null),
          order: stamp(Math.round(order)),
          live: stamp(true),
        },
      ]),
    );
    const a: Tree = Object.fromEntries(
      Object.entries(store.assignments).map(([id, target]) => [
        id,
        stamp(target),
      ]),
    );
    return { s, a };
  },
  project: projectSections,
  withinLimits: (p) =>
    (p.sections as unknown[]).length <= MAX_CHANNEL_SECTIONS &&
    Object.keys(p.assignments as object).length <=
      MAX_CHANNEL_SECTION_ASSIGNMENTS,
};
