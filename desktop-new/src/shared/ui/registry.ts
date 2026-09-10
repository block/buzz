export type ComponentStatus = "core" | "proposed";

/** Documentation location, independent from the source folder. */
export type ComponentCollection = "components" | "product-ui";

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
  select: { name: "Select", docs: "select", module: "@base-ui/react/select" },
  switch: { name: "Switch", docs: "switch", module: "@base-ui/react/switch" },
  tabs: { name: "Tabs", docs: "tabs", module: "@base-ui/react/tabs" },
  accordion: {
    name: "Accordion",
    docs: "accordion",
    module: "@base-ui/react/accordion",
  },
  dialog: { name: "Dialog", docs: "dialog", module: "@base-ui/react/dialog" },
  popover: {
    name: "Popover",
    docs: "popover",
    module: "@base-ui/react/popover",
  },
  previewCard: {
    name: "Preview Card",
    docs: "preview-card",
    module: "@base-ui/react/preview-card",
  },
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
  /** Where this component appears in the design-system navigation. */
  collection: ComponentCollection;
  owner?: string;
  /**
   * The component whose frame this component belongs inside in the design
   * system navigation. This is documentation hierarchy, not an import graph:
   * a child remains independently usable and documented on its own page.
   */
  parent?: string;
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
    slug: "flex-workspace",
    name: "Flex Layout",
    purpose:
      "Compare standard FlexLayout docking with the existing Dockview workspace. Blank panels with rounded tabs and Buzz colors; no custom drag handles or placement restrictions.",
    behavior: "FlexLayout tabs, docking, and resizing",
    variants: ["native docking"],
    status: "proposed",
    collection: "product-ui",
    owner: "desktop-new Workspace",
    source: "shared/ui/FlexWorkspace.tsx",
    baseUi: [],
    composes: ["button"],
  },
  {
    slug: "conversation",
    name: "Conversation",
    purpose:
      "A Session exchange: a prompt, stable private work areas, and shared replies. This is an interactive design study, not a connected runtime.",
    behavior:
      "Presentation-ready messages and ordered work areas; no fetching or sending",
    variants: [
      "two agents working",
      "one finished",
      "completed",
      "needs you",
      "failed",
      "waiting",
      "stopped",
      "unavailable",
    ],
    status: "proposed",
    collection: "product-ui",
    owner: "desktop-new Conversation",
    source: "features/conversation/ui/SessionExchange.tsx",
    baseUi: [],
    composes: ["conversation-message", "agent-work-area"],
  },
  {
    slug: "conversation-message",
    name: "Message",
    parent: "conversation",
    purpose:
      "An authored contribution with identity, time, content, actions, and delivery state. Agent answers never collapse into private work.",
    behavior: "Prepared content and caller-owned actions",
    variants: ["human", "agent", "actions", "failed send", "retry"],
    status: "proposed",
    collection: "product-ui",
    owner: "desktop-new Conversation",
    source: "features/conversation/ui/ConversationMessage.tsx",
    baseUi: [],
    composes: ["avatar", "button"],
  },
  {
    slug: "agent-work-area",
    name: "Agent work",
    parent: "conversation",
    purpose:
      "One agent’s owner-private work, with recent steps, expandable history, visible notices, and artifact access after completion.",
    behavior:
      "Berd-inspired disclosure anatomy; receives prepared activity, not raw events",
    variants: [
      "working",
      "completed",
      "needs you",
      "failed",
      "waiting",
      "stopped",
      "unavailable",
    ],
    status: "proposed",
    collection: "product-ui",
    owner: "desktop-new Agent Activity",
    source: "features/agent-activity/ui/AgentWorkArea.tsx",
    baseUi: [],
    composes: ["accordion", "avatar", "work-step"],
  },
  {
    slug: "work-step",
    name: "Work step",
    parent: "conversation",
    purpose:
      "A chronological progress, thought-summary, or tool step with optional expandable output.",
    behavior: "Tool output disclosure through Accordion",
    variants: ["progress", "thought", "tool", "running", "failed"],
    status: "proposed",
    collection: "product-ui",
    owner: "desktop-new Agent Activity",
    source: "features/agent-activity/ui/WorkStep.tsx",
    baseUi: [],
    composes: ["accordion"],
  },
  {
    slug: "conversation-state",
    name: "Conversation state",
    parent: "conversation",
    purpose:
      "Distinct loading, empty, failed, and read-only conversation states with explicit recovery.",
    behavior: "Caller supplies the state and retry action",
    variants: ["loading", "empty", "failed", "read-only"],
    status: "proposed",
    collection: "product-ui",
    owner: "desktop-new Conversation",
    source: "features/conversation/ui/ConversationState.tsx",
    baseUi: [],
    composes: ["button"],
  },

  {
    slug: "workspace",
    name: "DocView",
    purpose:
      "Arrange panels with continuous resizing and purple edge-drop highlights. Navigation stays anchored at the upper left; a panel may sit below it. Only splits that leave both panels usable are offered. No presets or tab stacking.",
    behavior: "Dockview pointer dragging and resize",
    variants: ["split-only"],
    status: "proposed",
    collection: "product-ui",
    owner: "desktop-new Workspace",
    source: "shared/ui/BentoWorkspace.tsx",
    baseUi: [],
    composes: ["panel-header"],
  },
  {
    slug: "switch",
    name: "Switch",
    purpose: "A labelled setting that is on or off.",
    behavior: "Base UI owns checked state and keyboard behavior",
    variants: ["off", "on", "disabled"],
    status: "proposed",
    collection: "components",
    owner: "desktop-new Design system",
    source: "shared/ui/Switch.tsx",
    baseUi: [BASE_UI_PARTS.switch],
    composes: [],
  },
  {
    slug: "button",
    name: "Button",
    purpose: "A labeled action with primary, quiet, or unfilled emphasis.",
    behavior: "Base UI Button",
    variants: ["primary", "quiet", "ghost"],
    status: "proposed",
    collection: "components",
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
    variants: [
      "quiet",
      "ghost",
      "solid",
      "tint",
      "chrome",
      "shape: control | round",
      "compact: 16px artwork, 30px target, Tabler stroke 2",
      "toolbar: 16px artwork, 32px target, Tabler stroke 2",
    ],
    status: "proposed",
    collection: "components",
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
    collection: "components",
    owner: "desktop-new Messages",
    source: "shared/ui/Avatar.tsx",
    baseUi: [BASE_UI_PARTS.avatar],
    composes: [],
  },
  {
    slug: "preview-card",
    name: "PreviewCard",
    purpose:
      "A portal-rendered, non-modal preview of an object's already-available context.",
    behavior: "Base UI Preview Card",
    variants: ["default"],
    status: "proposed",
    collection: "components",
    owner: "desktop-new Messages",
    source: "shared/ui/PreviewCard.tsx",
    baseUi: [BASE_UI_PARTS.previewCard],
    composes: [],
  },
  {
    slug: "inline-chip",
    name: "InlineChip",
    purpose:
      "A reference to a person, agent, channel, message, or link shown inline in a sentence.",
    behavior: "Semantic native button or image role",
    variants: [
      "person",
      "agent",
      "channel",
      "message",
      "link",
      "resolved preview",
      "loading",
      "unresolved",
    ],
    status: "proposed",
    collection: "components",
    owner: "desktop-new Messages",
    source: "shared/ui/InlineChip.tsx",
    baseUi: [],
    composes: ["preview-card"],
  },
  {
    slug: "panel",
    name: "Panel",
    purpose:
      "An independently legible workspace surface sitting on the atmospheric workspace backdrop. It owns only the rounded surface — fill, border, shadow, and clipping — never a header, content, or resize behaviour.",
    behavior: "Semantic native region",
    variants: ["panel"],
    status: "proposed",
    collection: "components",
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
    collection: "components",
    owner: "desktop-new Messages",
    source: "shared/ui/Tabs.tsx",
    baseUi: [BASE_UI_PARTS.tabs],
    composes: [],
  },
  {
    slug: "conversation-header",
    name: "ConversationHeader",
    purpose:
      "The orientation row for a message-bearing conversation surface. It composes PanelHeader with conversation identity, status, context, and feature-owned actions. Workspace tabs sit above it, never beside its controls.",
    behavior: "Semantic native header",
    variants: ["default"],
    status: "proposed",
    collection: "product-ui",
    owner: "desktop-new Messages",
    source: "shared/ui/ConversationHeader.tsx",
    baseUi: [],
    composes: ["panel-header"],
  },
  {
    slug: "panel-header",
    name: "PanelHeader",
    purpose:
      "A panel's header row: optional icon, title, and actions. It composes controls supplied through actions but owns neither the panel container nor the content below it.",
    behavior: "Semantic native header",
    variants: ["default", "compact"],
    status: "proposed",
    collection: "components",
    owner: "desktop-new Messages",
    parent: "panel",
    source: "shared/ui/PanelHeader.tsx",
    baseUi: [],
    composes: [],
  },
  {
    slug: "text-field",
    name: "TextField",
    purpose:
      "A labeled, single-line form control with optional supporting or error text.",
    behavior: "Base UI Field and Input",
    variants: [
      "inset",
      "soft",
      "text",
      "email",
      "password",
      "description",
      "error",
      "disabled",
    ],
    status: "proposed",
    collection: "components",
    owner: "desktop-new Onboarding",
    source: "shared/ui/TextField.tsx",
    baseUi: [BASE_UI_PARTS.field, BASE_UI_PARTS.input],
    composes: [],
  },
  {
    slug: "search-field",
    name: "SearchField",
    purpose: "A compact filter field with a search cue and clear action.",
    behavior: "Base UI Field and Input",
    variants: ["default"],
    status: "proposed",
    collection: "components",
    owner: "desktop-new Messages",
    source: "shared/ui/SearchField.tsx",
    baseUi: [BASE_UI_PARTS.field, BASE_UI_PARTS.input],
    composes: ["icon-button"],
  },
  {
    slug: "navigation-section",
    name: "NavigationSection",
    purpose: "A named group of related rows in a dense workspace navigator.",
    behavior: "Semantic native section",
    variants: ["default"],
    status: "proposed",
    collection: "components",
    owner: "desktop-new Messages",
    source: "shared/ui/NavigationSection.tsx",
    baseUi: [],
    composes: [],
  },
  {
    slug: "navigation-item",
    name: "NavigationItem",
    purpose: "A selectable destination row with optional icon and metadata.",
    behavior: "Base UI Button",
    variants: ["default", "inset", "selected"],
    status: "proposed",
    collection: "components",
    owner: "desktop-new Messages",
    source: "shared/ui/NavigationItem.tsx",
    baseUi: [BASE_UI_PARTS.button],
    composes: [],
  },
  {
    slug: "accordion",
    name: "Accordion",
    purpose: "Reveal related content without leaving the page.",
    behavior:
      "Base UI owns expansion, keyboard activation, and panel semantics",
    variants: ["default", "navigation", "activity", "controlled expansion"],
    status: "proposed",
    collection: "components",
    owner: "desktop-new Design system",
    source: "shared/ui/Accordion.tsx",
    baseUi: [BASE_UI_PARTS.accordion],
    composes: [],
  },
  {
    slug: "select",
    name: "Select",
    purpose: "A compact labelled choice with grouped options.",
    behavior: "Base UI owns focus, keyboard selection, grouping, and dismissal",
    variants: ["grouped"],
    status: "proposed",
    collection: "components",
    owner: "desktop-new Design system",
    source: "shared/ui/Select.tsx",
    baseUi: [BASE_UI_PARTS.select],
    composes: ["button"],
  },
  {
    slug: "composer",
    name: "Composer",
    purpose:
      "A conversation authoring surface that keeps a draft, accepts a message, and exposes contribution tools.",
    behavior: "Semantic native form and textarea",
    variants: [
      "ready",
      "multiline",
      "formatting",
      "sending",
      "failed",
      "disabled",
      "responding",
      "stopping",
    ],
    status: "proposed",
    collection: "product-ui",
    owner: "desktop-new Messages",
    source: "features/composer/ui/MessageComposer.tsx",
    baseUi: [],
    composes: ["icon-button", "composer-formatting-bar"],
  },
  {
    slug: "composer-expression-picker",
    name: "Emoji and GIF picker",
    purpose: "Search and insert emoji, custom emoji, or GIFs while composing.",
    behavior:
      "Composer-owned popover; seeded preview data until relay capability work lands",
    variants: ["emoji", "GIFs", "search", "custom emoji"],
    status: "proposed",
    collection: "product-ui",
    owner: "desktop-new Composer",
    parent: "composer",
    source: "features/composer/ui/EmojiPicker.tsx",
    baseUi: [BASE_UI_PARTS.popover],
    composes: ["icon-button", "tabs"],
  },
  {
    slug: "composer-formatting-bar",
    name: "Formatting bar",
    purpose:
      "Composer formatting controls that replace the ordinary ingress controls.",
    behavior:
      "Semantic control group; formatting commands await editor integration",
    variants: ["expanded", "unavailable"],
    status: "proposed",
    collection: "product-ui",
    owner: "desktop-new Composer",
    parent: "composer",
    source: "features/composer/ui/ComposerFormattingBar.tsx",
    baseUi: [],
    composes: ["icon-button"],
  },
  {
    slug: "activity-rail",
    name: "Activity rail",
    purpose:
      "A compact, readable status line for agent work that continues beside a conversation.",
    behavior: "Semantic native status",
    variants: ["working"],
    status: "proposed",
    collection: "product-ui",
    owner: "desktop-new Messages",
    parent: "composer",
    source: "features/agent-activity/ui/AgentActivityRail.tsx",
    baseUi: [],
    composes: [],
  },
  {
    slug: "onboarding",
    name: "Onboarding",
    purpose:
      "The account, identity, and profile moments that introduce Buzz on the product backdrop.",
    behavior: "Composes shared Buzz fields, actions, and panels",
    variants: [
      "create account",
      "welcome back",
      "create identity key",
      "build profile",
    ],
    status: "proposed",
    collection: "product-ui",
    owner: "desktop-new Onboarding",
    source: "features/onboarding/ui/OnboardingShowcase.tsx",
    baseUi: [],
    composes: ["text-field", "button", "icon-button", "panel"],
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
