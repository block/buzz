export type ComponentStatus = "core" | "proposed";

/**
 * A Base UI part a component is built on. `name` is the export as documented;
 * `docs` is the path segment on base-ui.com/react/components, and `module` is
 * the import specifier, so the two cannot be claimed independently of the code.
 */
export type BaseUiPart = {
  name: string;
  docs: string;
  module: string;
};

export const BASE_UI_PARTS = {
  avatar: { name: "Avatar", docs: "avatar", module: "@base-ui/react/avatar" },
  button: { name: "Button", docs: "button", module: "@base-ui/react/button" },
  field: { name: "Field", docs: "field", module: "@base-ui/react/field" },
  input: { name: "Input", docs: "input", module: "@base-ui/react/input" },
  tabs: { name: "Tabs", docs: "tabs", module: "@base-ui/react/tabs" },
  accordion: {
    name: "Accordion",
    docs: "accordion",
    module: "@base-ui/react/accordion",
  },
  dialog: { name: "Dialog", docs: "dialog", module: "@base-ui/react/dialog" },
} as const satisfies Record<string, BaseUiPart>;

export const BASE_UI_DOCS_ROOT = "https://base-ui.com/react/components";

export function baseUiDocsUrl(part: BaseUiPart): string {
  return `${BASE_UI_DOCS_ROOT}/${part.docs}`;
}

export type ComponentDefinition = {
  slug: string;
  name: string;
  purpose: string;
  behavior: string;
  variants: readonly string[];
  status: ComponentStatus;
  owner?: string;
  /** The file this component lives in, relative to `src/`. */
  source: string;
  /**
   * The Base UI parts this component imports itself. Empty means it is built
   * from native elements — which is a legitimate answer, not a gap: a header,
   * a section, and a surface have no behaviour to inherit.
   */
  baseUi: readonly BaseUiPart[];
  /**
   * Sibling components it composes, by slug. A component with no Base UI part
   * of its own can still inherit behaviour through one of these.
   */
  composes: readonly string[];
};

export const COMPONENTS: readonly ComponentDefinition[] = [
  {
    slug: "button",
    name: "Button",
    purpose: "A labeled action with primary, quiet, or unfilled emphasis.",
    behavior: "Base UI Button",
    variants: ["primary", "quiet", "ghost"],
    status: "proposed",
    owner: "desktop-new Messages",
    source: "shared/ui/Button.tsx",
    baseUi: [BASE_UI_PARTS.button],
    composes: [],
  },
  {
    slug: "icon-button",
    name: "IconButton",
    purpose: "A compact icon-only action that always owns an accessible label.",
    behavior: "Composes Buzz Button",
    variants: ["quiet", "ghost", "solid", "chrome"],
    status: "proposed",
    owner: "desktop-new Messages",
    source: "shared/ui/IconButton.tsx",
    baseUi: [],
    composes: ["button"],
  },
  {
    slug: "avatar",
    name: "Avatar",
    purpose: "A person or agent identity image with a stable fallback.",
    behavior: "Base UI Avatar",
    variants: ["small", "default", "large"],
    status: "proposed",
    owner: "desktop-new Messages",
    source: "shared/ui/Avatar.tsx",
    baseUi: [BASE_UI_PARTS.avatar],
    composes: [],
  },
  {
    slug: "inline-chip",
    name: "InlineChip",
    purpose:
      "A reference to a person, agent, channel, message, or link shown inline in a sentence.",
    behavior: "Semantic native button or image role",
    variants: ["person", "agent", "channel", "message", "link"],
    status: "proposed",
    owner: "desktop-new Messages",
    source: "shared/ui/InlineChip.tsx",
    baseUi: [],
    composes: [],
  },
  {
    slug: "panel",
    name: "Panel",
    purpose: "A major panel sitting on the atmospheric workspace backdrop.",
    behavior: "Semantic native region",
    variants: ["panel", "connected-left", "connected-right"],
    status: "proposed",
    owner: "desktop-new Messages",
    source: "shared/ui/Panel.tsx",
    baseUi: [],
    composes: [],
  },
  {
    slug: "tabs",
    name: "Tabs",
    purpose:
      "A single-select switch between sibling views. `chrome` is the glass pill for the app gradient; `panel` is an underline for a plain surface. One component because only the surface differs — the behaviour, keyboard model, and props are identical.",
    behavior: "Base UI Tabs",
    variants: ["chrome", "panel"],
    status: "proposed",
    owner: "desktop-new Messages",
    source: "shared/ui/Tabs.tsx",
    baseUi: [BASE_UI_PARTS.tabs],
    composes: [],
  },
  {
    slug: "panel-header",
    name: "PanelHeader",
    purpose:
      "The identity and action boundary at the top of a workspace panel.",
    behavior: "Semantic native header",
    variants: ["default", "compact"],
    status: "proposed",
    owner: "desktop-new Messages",
    source: "shared/ui/PanelHeader.tsx",
    baseUi: [],
    composes: [],
  },
  {
    slug: "search-field",
    name: "SearchField",
    purpose: "A compact filter field with a search cue and clear action.",
    behavior: "Base UI Field and Input",
    variants: ["default"],
    status: "proposed",
    owner: "desktop-new Messages",
    source: "shared/ui/SearchField.tsx",
    baseUi: [BASE_UI_PARTS.field, BASE_UI_PARTS.input],
    composes: ["icon-button"],
  },
  {
    slug: "navigator-section",
    name: "NavigatorSection",
    purpose: "A named group of related rows in a dense workspace navigator.",
    behavior: "Semantic native section",
    variants: ["default"],
    status: "proposed",
    owner: "desktop-new Messages",
    source: "shared/ui/NavigatorSection.tsx",
    baseUi: [],
    composes: [],
  },
  {
    slug: "navigator-row",
    name: "NavigatorRow",
    purpose: "A selectable destination row with optional icon and metadata.",
    behavior: "Composes Buzz Button",
    variants: ["default", "inset", "selected"],
    status: "proposed",
    owner: "desktop-new Messages",
    source: "shared/ui/NavigatorRow.tsx",
    baseUi: [],
    composes: ["button"],
  },
];

/**
 * What a component is actually built on, once composition is followed.
 *
 * `own` is what the file itself imports from Base UI. `inherited` is what it
 * gets through a sibling — IconButton has no Base UI import of its own, but it
 * renders a Buzz Button, so Base UI Button's behaviour is still underneath it.
 * Reporting only `own` would call that component unbacked, which is wrong; not
 * distinguishing them would hide where the dependency actually enters.
 */
export type BaseUiBacking = {
  own: readonly BaseUiPart[];
  inherited: readonly { part: BaseUiPart; through: string }[];
};

export function resolveBaseUiBacking(slug: string): BaseUiBacking {
  const byPart = new Map<string, { part: BaseUiPart; through: string }>();
  const root = COMPONENTS.find((candidate) => candidate.slug === slug);
  if (!root) return { own: [], inherited: [] };

  // Bounded by the registry itself, and `seen` makes a cycle terminate rather
  // than recurse — a component composing a component that composes it back is a
  // mistake we would rather see as a missing row than a hung page.
  const seen = new Set<string>([slug]);
  const queue = [...root.composes];

  while (queue.length > 0) {
    const nextSlug = queue.shift();
    if (!nextSlug || seen.has(nextSlug)) continue;
    seen.add(nextSlug);

    const next = COMPONENTS.find((candidate) => candidate.slug === nextSlug);
    if (!next) continue;

    for (const part of next.baseUi) {
      if (!byPart.has(part.name))
        byPart.set(part.name, { part, through: next.name });
    }
    queue.push(...next.composes);
  }

  // A part imported directly is reported once, as its own — not twice because a
  // composed sibling happens to use it too.
  for (const part of root.baseUi) byPart.delete(part.name);

  return { own: root.baseUi, inherited: [...byPart.values()] };
}
