export type ComponentStatus = "core" | "proposed";

export type ComponentDefinition = {
  slug: string;
  name: string;
  purpose: string;
  behavior: string;
  variants: readonly string[];
  status: ComponentStatus;
  owner?: string;
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
  },
  {
    slug: "icon-button",
    name: "IconButton",
    purpose: "A compact icon-only action that always owns an accessible label.",
    behavior: "Composes Buzz Button",
    variants: ["quiet", "ghost", "solid", "chrome"],
    status: "proposed",
    owner: "desktop-new Messages",
  },
  {
    slug: "avatar",
    name: "Avatar",
    purpose: "A person or agent identity image with a stable fallback.",
    behavior: "Base UI Avatar",
    variants: ["small", "default", "large"],
    status: "proposed",
    owner: "desktop-new Messages",
  },
  {
    slug: "workspace-surface",
    name: "WorkspaceSurface",
    purpose: "A major panel sitting on the atmospheric workspace backdrop.",
    behavior: "Semantic native region",
    variants: ["panel", "connected-left", "connected-right"],
    status: "proposed",
    owner: "desktop-new Messages",
  },
  {
    slug: "segmented-navigation",
    name: "SegmentedNavigation",
    purpose: "A single-select switch between top-level workspace destinations.",
    behavior: "Base UI Tabs",
    variants: ["default"],
    status: "proposed",
    owner: "desktop-new Messages",
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
  },
  {
    slug: "search-field",
    name: "SearchField",
    purpose: "A compact filter field with a search cue and clear action.",
    behavior: "Base UI Field and Input",
    variants: ["default"],
    status: "proposed",
    owner: "desktop-new Messages",
  },
  {
    slug: "navigator-section",
    name: "NavigatorSection",
    purpose: "A named group of related rows in a dense workspace navigator.",
    behavior: "Semantic native section",
    variants: ["default"],
    status: "proposed",
    owner: "desktop-new Messages",
  },
  {
    slug: "navigator-row",
    name: "NavigatorRow",
    purpose: "A selectable destination row with optional icon and metadata.",
    behavior: "Composes Buzz Button",
    variants: ["default", "inset", "selected"],
    status: "proposed",
    owner: "desktop-new Messages",
  },
];
