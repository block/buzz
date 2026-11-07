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

type SectionNode = {
  name?: Reg;
  icon?: Reg;
  order?: Reg;
  live?: Reg;
  parent?: Reg;
};

export type SectionsView = {
  sections: ChannelSection[];
  assignments: Record<string, string>;
};

type RankedSection = {
  id: string;
  name: string;
  icon: unknown;
  order: number;
  parent: unknown;
};

const val = (reg: unknown) => (isReg(reg) ? reg[2] : undefined);

/** Live sections by `(order, id)`, with their canonical `order`. */
function rankedSections(tree: Tree): RankedSection[] {
  const nodes = (tree.s ?? {}) as Record<string, SectionNode>;
  return Object.entries(nodes)
    .filter(([, n]) => val(n.live) === true && isString(val(n.name)))
    .map(([id, n]) => ({
      id,
      name: val(n.name) as string,
      icon: val(n.icon),
      order: Number(val(n.order) ?? 0),
      parent: val(n.parent),
    }))
    .sort(
      (a, b) => a.order - b.order || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0),
    );
}

/** Highest canonical `order` among live sections, or -1 when there are none. */
export const maxLiveOrder = (tree: Tree) =>
  Math.max(-1, ...rankedSections(tree).map((s) => s.order));

/**
 * Live sections in sidebar order — every top-level section followed by its
 * children — with `order` densely renumbered inside each group.
 *
 * The sidebar nests one level. A `parent` register counts only when it names a
 * live section that is itself top-level, so a parent that is missing, deleted,
 * self-referential, or already nested leaves the section at the top level: a
 * cycle or a deeper chain flattens instead of hiding channels.
 */
function liveSections(tree: Tree): ChannelSection[] {
  const ranked = rankedSections(tree);
  const byId = new Map(ranked.map((section) => [section.id, section]));
  const liveParent = (section: RankedSection) => {
    const parentId = isString(section.parent)
      ? (section.parent as string)
      : undefined;
    return parentId && parentId !== section.id ? byId.get(parentId) : undefined;
  };
  const toSection = (
    section: RankedSection,
    order: number,
    parentId?: string,
  ): ChannelSection => ({
    id: section.id,
    name: section.name,
    ...(isString(section.icon) && section.icon
      ? { icon: section.icon as string }
      : {}),
    order,
    ...(parentId ? { parentId } : {}),
  });

  const roots: RankedSection[] = [];
  const childrenByParent = new Map<string, RankedSection[]>();
  for (const section of ranked) {
    const parent = liveParent(section);
    // A nested parent would make this section a grandchild: keep it top-level.
    if (!parent || liveParent(parent)) {
      roots.push(section);
      continue;
    }
    const siblings = childrenByParent.get(parent.id);
    if (siblings) siblings.push(section);
    else childrenByParent.set(parent.id, [section]);
  }

  return roots.flatMap((root, order) => [
    toSection(root, order),
    ...(childrenByParent.get(root.id) ?? []).map((child, childOrder) =>
      toSection(child, childOrder, root.id),
    ),
  ]);
}

/** Live ids sharing one parent (`undefined` = top level), in sidebar order. */
export const siblingIds = (
  sections: ChannelSection[],
  parentId: string | undefined,
): string[] =>
  sections
    .filter((section) => section.parentId === parentId)
    .map((section) => section.id);

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
        parent: (v) => v === null || isString(v),
      },
    },
    a: { "*": (v) => v === null || isString(v) },
  },
  fromLegacy(json, stamp) {
    const store = parseChannelSectionPayload(json);
    if (!store) return null;
    const s: Tree = Object.fromEntries(
      store.sections.map(({ id, name, icon, order, parentId }) => [
        id,
        {
          name: stamp(name),
          icon: stamp(icon ?? null),
          order: stamp(Math.round(order)),
          live: stamp(true),
          parent: stamp(parentId ?? null),
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
