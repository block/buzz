/**
 * The token registry.
 *
 * This is the machine-readable description of the colour system: what exists,
 * what each name is for, what it points at, and whether it is core or proposed.
 * The design system pages render from this, so a token added here appears in the
 * documentation automatically and the docs cannot drift from the system.
 *
 * Adding to this file is a normal, unreviewed action — see the growth procedure
 * in DESIGN.md. Every entry needs a `use` sentence and, if proposed, an `owner`.
 */

/** Whether a token is part of the vetted system or someone's addition. */
export type TokenStatus = "core" | "proposed" | "deprecated";

/** A private ramp step. Components never reference these. */
export interface RampStep {
  /** Position on the ramp. The number means a job, not a brightness. */
  step: number;
  /** What this step is for. */
  job: string;
  /** The CSS custom property, without `var()`. */
  variable: string;
}

/** A private ramp. */
export interface Ramp {
  id: string;
  name: string;
  /** Why this ramp exists and how it behaves. */
  description: string;
  steps: RampStep[];
  /** Rendered over the app backdrop so translucency is visible. */
  translucent?: boolean;
}

/** A public role. The only layer a screen may use. */
export interface Role {
  /** The Tailwind class, e.g. `bg-panel`. */
  token: string;
  /** The CSS custom property backing it. */
  variable: string;
  /** What it points at, for display: `neutral 1`, or a note for exceptions. */
  pointsAt: string;
  /** One sentence: when to use this. */
  use: string;
  status: TokenStatus;
  /** Required when status is `proposed`. */
  owner?: string;
  /** Set when the value is a literal rather than a ramp reference. */
  exception?: string;
}

export interface RoleGroup {
  id: string;
  name: string;
  description: string;
  roles: Role[];
}

/* ============================================================
   PRIVATE RAMPS
   ============================================================ */

const NEUTRAL_JOBS = [
  "app background",
  "subtle background",
  "component background",
  "component hover",
  "component selected",
  "subtle border",
  "border",
  "border hover",
  "solid fill",
  "solid fill hover",
  "dark fill",
  "high-contrast text",
];

/** The five accent steps any role points at. The rest exist so the ramp is
 *  complete and interpolation stays even. */
const IDENTITY_STEPS: Array<[number, string]> = [
  [2, "tinted surface"],
  [3, "tint hover / selected"],
  [8, "border, focus ring"],
  [9, "solid fill"],
  [11, "accent text"],
];

function identityRamp(id: string, name: string, description: string): Ramp {
  return {
    id,
    name,
    description,
    steps: IDENTITY_STEPS.map(([step, job]) => ({
      step,
      job,
      variable: `--${id}-${step}`,
    })),
  };
}

export const RAMPS: Ramp[] = [
  {
    id: "neutral",
    name: "Neutral",
    description:
      "Twelve steps, each step a fixed job. The number means a job, not a brightness — which is what lets a role encode a position that stays true across every palette.",
    steps: NEUTRAL_JOBS.map((job, i) => ({
      step: i + 1,
      job,
      variable: `--neutral-${i + 1}`,
    })),
  },
  identityRamp(
    "accent",
    "Accent — a slot, not a colour",
    "Nothing above this ramp knows the hue, so the accent can change, become a person's preference, or vary per theme without a single component changing. Step 9 barely moves between modes because a saturated fill reads on white and on near-black; step 11 inverts direction to stay readable.",
  ),
  identityRamp(
    "danger",
    "Danger",
    "The same five jobs as accent. Learning one family teaches all of them.",
  ),
  identityRamp("success", "Success", "The same five jobs as accent."),
  identityRamp("warning", "Warning", "The same five jobs as accent."),
  identityRamp("info", "Info", "The same five jobs as accent."),
  {
    id: "glass",
    name: "Glass",
    description:
      "A translucency ramp of fills. Each step is the mode's surface colour at an increasing opacity, which is what lets a hover move one step up the ramp instead of holding its own literal. Fills only — a glass surface's bright rim is a border, and it is a documented exception.",
    translucent: true,
    steps: [
      { step: 1, job: "barely there", variable: "--glass-1" },
      { step: 2, job: "quiet glass", variable: "--glass-2" },
      { step: 3, job: "default glass", variable: "--glass-3" },
      { step: 4, job: "glass hovered", variable: "--glass-4" },
      { step: 5, job: "nearly solid", variable: "--glass-5" },
    ],
  },
];

/* ============================================================
   PUBLIC ROLES
   ============================================================ */

function identityGroup(
  id: string,
  label: string,
  description: string,
): RoleGroup {
  return {
    id,
    name: label,
    description,
    roles: [
      {
        token: `bg-${id}`,
        variable: `--bg-${id}`,
        pointsAt: `${id} 9`,
        use: "The solid fill: primary buttons, active toggles. Takes its paired text.",
        status: "core",
      },
      {
        token: `text-on-${id}`,
        variable: `--text-on-${id}`,
        pointsAt: "computed from the fill's lightness",
        use: `Text sitting on bg-${id}.`,
        status: "core",
        exception:
          "Computed rather than fixed: white is readable on a blue or purple fill and unreadable on yellow or lime.",
      },
      {
        token: `text-${id}`,
        variable: `--text-${id}`,
        pointsAt: `${id} 11`,
        use: "Coloured text on a neutral background: links, labels.",
        status: "core",
      },
      {
        token: `bg-${id}-tint`,
        variable: `--bg-${id}-tint`,
        pointsAt: `${id} 2`,
        use: "A tinted surface carrying meaning: chips, callouts, selected rows. Takes coloured text.",
        status: "core",
      },
      {
        token: `bg-${id}-tint-hover`,
        variable: `--bg-${id}-tint-hover`,
        pointsAt: `${id} 3`,
        use: "That tinted surface hovered or selected.",
        status: "core",
      },
      {
        token: `border-${id}`,
        variable: `--border-${id}`,
        pointsAt: `${id} 8`,
        use: "Focus rings and active borders.",
        status: "core",
      },
    ],
  };
}

export const ROLE_GROUPS: RoleGroup[] = [
  {
    id: "surfaces",
    name: "Structural surfaces",
    description:
      "Named and closed, because there are only a few right answers and guessing produces a broken interface. Ask one question: is it behind, on, above, or in?",
    roles: [
      {
        token: "bg-app",
        variable: "--bg-app",
        pointsAt: "gradient-1",
        use: "The backdrop everything sits on.",
        status: "core",
      },
      {
        token: "bg-panel",
        variable: "--bg-panel",
        pointsAt: "neutral 1 light / neutral 3 dark",
        use: "Everything sitting on the backdrop: the navigation column, content, timelines, cards, rows.",
        status: "core",
      },
      {
        token: "bg-float",
        variable: "--bg-float",
        pointsAt: "neutral 1 light / neutral 5 dark",
        use: "Anything hovering above the page: menus, dialogs, tooltips, toasts. Shares a light value with bg-panel and diverges in dark, because a shadow cannot carry elevation on a near-black background.",
        status: "core",
      },
      {
        token: "bg-inset",
        variable: "--bg-inset",
        pointsAt: "neutral 3 light / neutral 2 dark",
        use: "Anything pushed in: inputs, code blocks, quotes.",
        status: "core",
      },
      {
        token: "bg-hover",
        variable: "--bg-hover",
        pointsAt: "neutral 4",
        use: "Any neutral row or item under the cursor. Region-less — it works on any neutral surface, which is why there is only one of them.",
        status: "core",
      },
    ],
  },
  {
    id: "emphasis",
    name: "Emphasis",
    description:
      "One three-level ramp shared by text and borders: normal, lesser, really lesser. States that are not levels of emphasis get their own names rather than extending the ramp.",
    roles: [
      {
        token: "text-primary",
        variable: "--text-primary",
        pointsAt: "neutral 12",
        use: "Normal reading text.",
        status: "core",
      },
      {
        token: "text-secondary",
        variable: "--text-secondary",
        pointsAt: "neutral 10",
        use: "Supporting text: author names, labels.",
        status: "core",
      },
      {
        token: "text-tertiary",
        variable: "--text-tertiary",
        pointsAt: "neutral 9",
        use: "Metadata: timestamps, counts, hints.",
        status: "core",
      },
      {
        token: "text-disabled",
        variable: "--text-disabled",
        pointsAt: "neutral 8",
        use: "Unavailable. A state, not a fourth level of the ramp.",
        status: "core",
      },
      {
        token: "border-primary",
        variable: "--border-primary",
        pointsAt: "neutral 7",
        use: "The default visible line.",
        status: "core",
      },
      {
        token: "border-secondary",
        variable: "--border-secondary",
        pointsAt: "neutral 6",
        use: "A lighter separator inside a group.",
        status: "core",
      },
      {
        token: "border-tertiary",
        variable: "--border-tertiary",
        pointsAt: "neutral 5",
        use: "The faintest line.",
        status: "core",
      },
    ],
  },
  identityGroup(
    "accent",
    "Accent",
    "Two arrangements and no third: a solid fill with paired text on it, or a tint fill with coloured text on it. There is deliberately nothing between them.",
  ),
  {
    id: "inverse",
    name: "Inverse",
    description:
      "A high-contrast fill for surfaces that must stand apart from everything: tooltips, toasts.",
    roles: [
      {
        token: "bg-inverse",
        variable: "--bg-inverse",
        pointsAt: "neutral 11",
        use: "High-contrast fill: tooltips, toasts.",
        status: "core",
      },
      {
        token: "bg-inverse-hover",
        variable: "--bg-inverse-hover",
        pointsAt: "neutral 10",
        use: "Its hover. Step 10 sits on the light side of step 11 in the light ramp and the dark side in the dark ramp, so the hover inverts direction with no special-casing.",
        status: "core",
      },
      {
        token: "text-on-inverse",
        variable: "--text-on-inverse",
        pointsAt: "computed from the fill's lightness",
        use: "Text sitting on bg-inverse.",
        status: "core",
        exception: "Computed rather than fixed.",
      },
    ],
  },
  identityGroup("danger", "Danger", "Destructive actions and error states."),
  identityGroup("success", "Success", "Confirmation and healthy states."),
  identityGroup("warning", "Warning", "Caution that is not yet an error."),
  identityGroup("info", "Info", "Neutral information and guidance."),
  {
    id: "material",
    name: "Material",
    description:
      "A translucent blurred surface is the same region in a different material. Translucency and blur live inside the value; a component never assembles them from a fill, a transparency, and a blur amount.",
    roles: [
      {
        token: "bg-chrome-glass",
        variable: "--bg-chrome-glass",
        pointsAt: "glass 3 + blur-md",
        use: "The top bar and floating chrome.",
        status: "core",
      },
      {
        token: "bg-chrome-glass-hover",
        variable: "--bg-chrome-glass-hover",
        pointsAt: "glass 4 + blur-md",
        use: "A chrome control under the cursor. A glass hover moves one step up the ramp, changing opacity and never blur.",
        status: "core",
      },
      {
        token: "bg-chrome-selected",
        variable: "--bg-chrome-selected",
        pointsAt: "neutral 1 light / neutral 5 dark",
        use: "The selected item inside chrome. Carries no material suffix because omitting it means opaque — on glass, elevation reads as less translucency, not a lighter colour.",
        status: "core",
      },
      {
        token: "bg-panel-glass",
        variable: "--bg-panel-glass",
        pointsAt: "glass 2 + blur-lg",
        use: "A panel that should let the backdrop through.",
        status: "core",
      },
      {
        token: "bg-float-glass",
        variable: "--bg-float-glass",
        pointsAt: "glass 3 + blur-md",
        use: "A floating surface that should let content through.",
        status: "core",
      },
    ],
  },
  {
    id: "focus",
    name: "Focus",
    description: "Focus is part of the design, never an artefact to suppress.",
    roles: [
      {
        token: "ring-focus",
        variable: "--ring-focus",
        pointsAt: "accent 8",
        use: "The keyboard focus ring.",
        status: "core",
      },
    ],
  },
  {
    id: "categorical",
    name: "Categorical tints",
    description:
      "The one documented exception to naming colours after their job: telling two projects apart genuinely is a choice about appearance, and pretending otherwise would push people back to raw values.",
    roles: (["blue", "purple", "orange", "green", "red", "cyan"] as const).map(
      (hue) => ({
        token: `bg-tint-${hue}`,
        variable: `--tint-${hue}`,
        pointsAt: "a literal, by design",
        use: `A categorical ${hue} tint for distinguishing one thing from another.`,
        status: "core" as TokenStatus,
        exception: "Named by appearance because the choice is appearance.",
      }),
    ),
  },
];

/* ============================================================
   THE VOCABULARY
   Every token in the system is built from these words. A new name
   combining existing words is routine; a new WORD is what the
   audit reports on its own line.
   ============================================================ */

export const VOCABULARY: Array<{ group: string; words: string[] }> = [
  { group: "property", words: ["bg", "text", "border", "ring"] },
  { group: "region", words: ["app", "panel", "float", "chrome", "inset"] },
  { group: "emphasis", words: ["primary", "secondary", "tertiary"] },
  { group: "state", words: ["hover", "selected", "disabled"] },
  { group: "material", words: ["glass"] },
  { group: "modifier", words: ["tint"] },
  { group: "identity", words: ["accent", "inverse"] },
  { group: "status", words: ["success", "warning", "danger", "info"] },
  { group: "paired", words: ["on-accent", "on-inverse"] },
  {
    group: "categorical",
    words: [
      "tint-blue",
      "tint-purple",
      "tint-orange",
      "tint-green",
      "tint-red",
      "tint-cyan",
    ],
  },
];

export const GRAMMAR = "<property>-<role>[-<modifier>][-<material>][-<state>]";

/** Fixed order, so there is only one correct spelling. */
export const GRAMMAR_EXAMPLES = {
  legal: ["bg-chrome-glass-hover", "bg-accent-tint-hover", "text-secondary"],
  illegal: ["bg-chrome-hover-glass", "bg-hover-chrome"],
};

/** Runs per change, by whoever needs the value. Nothing here needs permission. */
export const GROWTH_PROCEDURE = [
  "Search the role list by intent, not by colour.",
  "A state of an existing role — add the -hover, -selected, or -disabled sibling with both values.",
  "A material variant of an existing role — add the -glass sibling with both values and its blur token.",
  "A new role using existing words — add the name, both values, a one-sentence description, and an owner.",
  "A new hue — generate its ramp, add roles pointing at steps. Never a literal.",
  "A new vocabulary word — allowed, but it is the thing the audit reports on its own line, so use an existing word if one fits.",
  "Never write a raw value. If nothing above applies, say so rather than reaching for a literal.",
];

/* ============================================================
   TYPOGRAPHY
   ============================================================ */

/** A public type role. Carries its whole setting, because size, line height,
 *  tracking, and weight are one decision rather than four. */
export interface TypeRole {
  /** The Tailwind class, e.g. `text-body`. */
  token: string;
  /** Step on the size ramp this points at, for display. */
  pointsAt: string;
  /** Rendered size at the default preference and zoom, for display only —
   *  never a value a component may use. */
  size: string;
  lineHeight: string;
  tracking: string;
  weight: string;
  /** One sentence: when to use this. */
  use: string;
  status: TokenStatus;
  /** Set when the role renders in the mono face. */
  mono?: boolean;
}

/** Ordered loudest to quietest, which is also the order they are chosen in. */
export const TYPE_ROLES: TypeRole[] = [
  {
    token: "text-display",
    pointsAt: "size 8",
    size: "32px",
    lineHeight: "1.2",
    tracking: "-0.024em",
    weight: "400",
    use: "Onboarding and empty states — a screen with nothing on it yet, or one asking a single question. Most screens have none.",
    status: "core",
  },
  {
    token: "text-title",
    pointsAt: "size 7",
    size: "24px",
    lineHeight: "1.2",
    tracking: "-0.019em",
    weight: "400",
    use: "The headline inside content: a feed card, a thread topic. At this size the size carries it, which is why the weight stays at 400.",
    status: "core",
  },
  {
    token: "text-heading",
    pointsAt: "size 6",
    size: "16px",
    lineHeight: "1.35",
    tracking: "-0.011em",
    weight: "600",
    use: "The name of the thing you are looking at: a channel, a panel, a section. Same size as body-lg, separated from it by weight alone.",
    status: "core",
  },
  {
    token: "text-body-lg",
    pointsAt: "size 6",
    size: "16px",
    lineHeight: "1.6",
    tracking: "-0.011em",
    weight: "400",
    use: "Reading columns and introductions, where a paragraph is the point rather than a description of something else.",
    status: "core",
  },
  {
    token: "text-body",
    pointsAt: "size 4",
    size: "14px",
    lineHeight: "1.5",
    tracking: "-0.006em",
    weight: "400",
    use: "The default, and 90% of the product. If you are unsure, this is it.",
    status: "core",
  },
  {
    token: "text-body-sm",
    pointsAt: "size 2",
    size: "12px",
    lineHeight: "1.5",
    tracking: "0em",
    weight: "400",
    use: "Secondary text: descriptions, timestamps, chip and tab labels, help text under a control. The smallest text in the product.",
    status: "core",
  },
  {
    token: "text-mono-lg",
    pointsAt: "size 5",
    size: "15px",
    lineHeight: "1.5",
    tracking: "-0.009em",
    weight: "400",
    use: "Codes and keys a person has to transcribe or read aloud. Paired with body-lg.",
    status: "core",
    mono: true,
  },
  {
    token: "text-mono",
    pointsAt: "size 3",
    size: "13px",
    lineHeight: "1.5",
    tracking: "-0.003em",
    weight: "400",
    use: "Inline code, pubkeys, paths, branch names, hex values — anything where the characters matter individually. Paired with body.",
    status: "core",
    mono: true,
  },
  {
    token: "text-mono-sm",
    pointsAt: "size 1",
    size: "11px",
    lineHeight: "1",
    tracking: "0.005em",
    weight: "400",
    use: "Terminal tabs, code-block headers, dense chrome. Nothing read in quantity. Paired with body-sm.",
    status: "core",
    mono: true,
  },
];

/** The two faces. Both already shipped in every current Buzz client. */
export const TYPE_FAMILIES = [
  {
    token: "font-sans",
    name: "Inter Variable",
    use: "Everything. Drawn for interface text at small sizes, and already the sans in desktop, web, and mobile.",
  },
  {
    token: "font-mono",
    name: "JetBrains Mono",
    use: "Code, keys, and identifiers. Already the mono in the existing client's terminal.",
  },
];

/** The private ramps a type role points at. Components never reference these. */
export const TYPE_RAMPS = [
  {
    id: "size",
    name: "Size",
    description:
      "Eight steps, sized from the product rather than composed. Across 73 real Buzz screens, 90% of all text is one size and two sizes cover 93%; 16/18/20/22 together were 1.5%, scattered and inconsistent. So the ramp is deliberately short and the gap between 16 and 24 is deliberately empty — an app does not have a document outline. Mono sits one step below its sans partner throughout. Every value derives from a virtual rem, so the whole ramp follows the person's font-size preference and keyboard zoom.",
    steps: [
      { step: 1, job: "mono small", value: "11px" },
      { step: 2, job: "body small", value: "12px" },
      { step: 3, job: "mono", value: "13px" },
      { step: 4, job: "body — the default", value: "14px" },
      { step: 5, job: "mono large", value: "15px" },
      { step: 6, job: "body large, heading", value: "16px" },
      { step: 7, job: "title", value: "24px" },
      { step: 8, job: "display", value: "32px" },
    ],
  },
  {
    id: "tracking",
    name: "Tracking",
    description:
      "An optical correction, not a style. Inter needs progressively tighter spacing as it grows — tracking that looks correct at 14px looks loose at 32px — so the ramp is named per size step rather than per role, and the correction follows the size it corrects. Two roles at 16px therefore get the same tracking by construction. It turns slightly positive at the smallest step, where letters need air to stay legible.",
    steps: [
      { step: 8, job: "32px", value: "-0.024em" },
      { step: 7, job: "24px", value: "-0.019em" },
      { step: 6, job: "16px", value: "-0.011em" },
      { step: 5, job: "15px", value: "-0.009em" },
      { step: 4, job: "14px", value: "-0.006em" },
      { step: 3, job: "13px", value: "-0.003em" },
      { step: 2, job: "12px", value: "0em" },
      { step: 1, job: "11px", value: "0.005em" },
    ],
  },
  {
    id: "weight",
    name: "Weight",
    description:
      "Two values with two jobs, not a ramp. 400 is content — everything read. 600 is structure and emphasis: the thing that names what you are looking at, or the words a sentence leans on. 500 was rendered as the marker for a selected channel, an active tab, and an unread row, and does not read as intent in a scanned list — a sub-pixel stem difference at body size, yet heavy enough to muddy a column. 700 is louder than this product needs. State is said with colour, a fill, or a dot.",
    steps: [
      { step: 400, job: "content", value: "400" },
      { step: 600, job: "structure and emphasis", value: "600" },
    ],
  },
];

export const ELEVATION = [
  {
    token: "shadow-xs",
    variable: "--shadow-xs",
    use: "The default lift: a selected pill, a small raised control.",
  },
  {
    token: "shadow-sm",
    variable: "--shadow-sm",
    use: "A floating surface: menus, dialogs, popovers.",
  },
];

export const BLUR = [
  { token: "blur-sm", variable: "--blur-sm", value: "8px" },
  { token: "blur-md", variable: "--blur-md", value: "16px" },
  { token: "blur-lg", variable: "--blur-lg", value: "32px" },
];

/** The entire exception list. Everything else points at a ramp step. */
export const EXCEPTIONS = [
  {
    name: "text-on-accent, text-on-inverse, and the four status pairings",
    why: "Computed from their fill's lightness rather than fixed, because white is readable on a blue or purple fill and unreadable on yellow or lime. This is what keeps a free choice of accent hue from becoming a contrast lottery.",
  },
  {
    name: "--rim-lit, --rim-shade",
    why: "The glass rim is a directional light effect, not a solid line. It is not on the glass ramp — that is a ramp of fills, and in dark mode the fill is translucent near-black while the rim stays translucent white. It is not one of the numbered gradients either: those are background treatments, this is a material detail.",
  },
  {
    name: "gradient-1, texture-dots",
    why: "Not colours in the ramp sense.",
  },
  {
    name: "the categorical tints",
    why: "Named by appearance because the choice is appearance.",
  },
];
