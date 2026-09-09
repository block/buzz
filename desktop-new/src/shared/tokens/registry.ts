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

/**
 * What each of the twelve steps is *for*, as a starting point.
 *
 * The same twelve jobs in every hue is what makes a twelve-step scale useful and
 * the step-to-role mapping deterministic: a role picks a step number, not a
 * colour.
 *
 * **These are the scale's general intentions, not a record of Buzz's decisions,
 * and the two have already diverged.** The list came with the values and said
 * step 4 was "component hover" and step 6 "subtle border" — while in this product
 * step 4 is the one border weight and step 6 has one reader on a documentation
 * page. A generic label that contradicts the product is worse than none, because
 * a designer reading `/design` takes it for a decision someone made.
 *
 * So: **the label answers "what is this step generally for", and the neutral
 * ramp's own comments in `tokens.css` answer "what does Buzz actually do with
 * it".** When a step's real use settles into something durable, move it here.
 * `pnpm census` lists the real readers of every role.
 */
const STEP_JOBS = [
  "lightest surface",
  "subtle surface",
  "tinted surface",
  "tinted surface, hovered",
  "quiet border, or a selected surface",
  "border",
  "stronger border",
  "border hover, focus ring",
  "solid fill",
  "solid fill, hovered",
  "text on a tint, or an inverse fill",
  "high-contrast text",
];

/** A palette hue: twelve steps, authored per mode. The bottom layer. */
export interface PaletteHue {
  id: string;
  /** Which identity, if any, currently draws from this hue. */
  usedBy?: string;
  steps: Array<{ step: number; variable: string; job: string }>;
}

/**
 * A private, named backdrop treatment.
 *
 * Unlike a colour ramp, a treatment is a complete visual composition: several
 * colours plus their positions, falloff, and sometimes a vignette. Its name is
 * deliberately mode-specific — Night garden is the dark counterpart of Sky
 * field, not Sky field with its brightness turned down.
 */
export interface BackdropTreatment {
  id: string;
  name: string;
  variable: string;
  mode: "light" | "dark";
  pairedWith: string;
  description: string;
}

/**
 * Every hue in the palette.
 *
 * The bottom layer, and the only place a literal colour lives.
 * Values are Radix Colors (MIT), transcribed rather than depended on — Radix is
 * not on Block's Tech Radar, so this is a values-only copy with no package.
 *
 * It exists for two reasons. The tokens above it were 114 hand-picked hex
 * values with nothing enforcing that two tokens doing the same job agreed, and
 * they drifted. And a hue's dark steps are not its light steps dimmed: reaching
 * for a subtler dark purple by writing `purple-950/50` in a component put a real
 * colour decision somewhere it could not be named, paired, or measured.
 */
export const PALETTE: PaletteHue[] = [
  { id: "neutral", usedBy: "structure", steps: [] },
  { id: "purple", usedBy: "accent", steps: [] },
  { id: "red", usedBy: "danger", steps: [] },
  { id: "green", usedBy: "success", steps: [] },
  { id: "amber", usedBy: "warning", steps: [] },
  { id: "blue", usedBy: "info", steps: [] },
  { id: "cyan", steps: [] },
  { id: "orange", steps: [] },
].map((hue) => ({
  ...hue,
  steps: Array.from({ length: 12 }, (_, i) => ({
    step: i + 1,
    variable: `--${hue.id}-${i + 1}`,
    job: STEP_JOBS[i],
  })),
}));

export const BACKDROP_TREATMENTS: BackdropTreatment[] = [
  {
    id: "sky-field",
    name: "Sky field",
    variable: "--gradient-sky-field",
    mode: "light",
    pairedWith: "night-garden",
    description: "Blue sky, fresh green, and a sunlit yellow edge.",
  },
  {
    id: "peach-field",
    name: "Peach field",
    variable: "--gradient-peach-field",
    mode: "light",
    pairedWith: "signal-flare",
    description: "A bright peach wash with a cool blue edge.",
  },
  {
    id: "blue-hour",
    name: "Blue hour",
    variable: "--gradient-blue-hour",
    mode: "light",
    pairedWith: "electric-dusk",
    description: "Cyan, periwinkle, and a quiet lavender horizon.",
  },
  {
    id: "orchid-field",
    name: "Orchid field",
    variable: "--gradient-orchid-field",
    mode: "light",
    pairedWith: "ultraviolet",
    description: "Lilac, periwinkle, and a low orchid-pink bloom.",
  },
  {
    id: "night-garden",
    name: "Night garden",
    variable: "--gradient-night-garden",
    mode: "dark",
    pairedWith: "sky-field",
    description: "Deep teal atmosphere, emerald high light, and blue below.",
  },
  {
    id: "signal-flare",
    name: "Signal flare",
    variable: "--gradient-signal-flare",
    mode: "dark",
    pairedWith: "peach-field",
    description:
      "A warm ember field with orange high light and a rose horizon.",
  },
  {
    id: "electric-dusk",
    name: "Electric dusk",
    variable: "--gradient-electric-dusk",
    mode: "dark",
    pairedWith: "blue-hour",
    description: "Cyan and ultraviolet light over a deep blue atmosphere.",
  },
  {
    id: "ultraviolet",
    name: "Ultraviolet",
    variable: "--gradient-ultraviolet",
    mode: "dark",
    pairedWith: "orchid-field",
    description: "A deep violet field, blue high light, and a low rose flare.",
  },
];

/** The four stable appearance slots, each pairing one light and dark scene. */
export const BACKDROP_CHOICES = [
  {
    token: "gradient-1",
    variable: "--gradient-1",
    lightTreatment: "sky-field",
    darkTreatment: "night-garden",
    use: "The default app backdrop: Sky field in light mode, Night garden in dark.",
  },
  {
    token: "gradient-2",
    variable: "--gradient-2",
    lightTreatment: "peach-field",
    darkTreatment: "signal-flare",
    use: "A warm paired app backdrop: Peach field in light mode, Signal flare in dark.",
  },
  {
    token: "gradient-3",
    variable: "--gradient-3",
    lightTreatment: "blue-hour",
    darkTreatment: "electric-dusk",
    use: "A cool paired app backdrop: Blue hour in light mode, Electric dusk in dark.",
  },
  {
    token: "gradient-4",
    variable: "--gradient-4",
    lightTreatment: "orchid-field",
    darkTreatment: "ultraviolet",
    use: "A violet paired app backdrop: Orchid field in light mode, Ultraviolet in dark.",
  },
] as const;

export const RAMPS: Ramp[] = [
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

/**
 * There is no identity-group generator any more, and that is the point.
 *
 * A helper here built five roles from one line of a hue lookup — `bg-accent`,
 * `bg-accent-tint`, `text-accent`, and so on — which is exactly how twenty status
 * roles came to exist without anyone designing them. Every one of those roles
 * held the same palette step in both modes, so each was a name in front of a
 * number.
 *
 * Screens now write the step: `bg-purple-9`, `bg-purple-3`, `text-purple-12`.
 * Safe here and not in Tailwind because **every step is authored per mode**, so a
 * class still behaves in light and dark. The step-to-role mapping in DESIGN.md
 * survives as guidance for *which* step to reach for; it is no longer a
 * generator.
 *
 * A name comes back when a repeated accent PATTERN appears — a tinted callout on
 * four screens — and it will be named for the pattern, not the colour.
 */

export const ROLE_GROUPS: RoleGroup[] = [
  {
    id: "surfaces",
    name: "Structural surfaces",
    description:
      "The roles that exist because a ramp step cannot say them: each takes a different step in light and dark, so no single class like `bg-neutral-1` is correct in both. That is the whole test for whether a colour earns a name. Ask one question: is it behind, on, above, or in? (`bg-hover` was here and is now written as `bg-neutral-4` \u2014 it was the same step in both modes. Hover is a *relationship*, one step more contrast than whatever is underneath, which no single token could express anyway.)",
    roles: [
      {
        token: "bg-app",
        variable: "--bg-app",
        pointsAt: "gradient-1 (Sky field light / Night garden dark)",
        use: "The backdrop everything sits on. It takes the first paired appearance choice by default.",
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
        token: "bg-field-soft",
        variable: "--bg-field-soft",
        pointsAt: "neutral 2 light / neutral 4 dark",
        use: "The quiet neutral fill for soft form controls on a panel.",
        status: "proposed",
        owner: "desktop-new TextField",
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
        pointsAt: "neutral 4",
        use: "Every deliberate line: panel boundaries, dividers, separators. One quiet weight; add another only when a design proves a different boundary needs it.",
        status: "core",
      },
    ],
  },
  {
    id: "material",
    name: "Material",
    description:
      "Glass is applied as a whole material — a fill, a blur, a rim, and sometimes a lift — through the `glass-primary` and `glass-secondary` utilities. The fills below are what those utilities read; they are deliberately not registered as Tailwind colour utilities, because a bare `bg-glass-primary` class would be the fill without the rest, which is the failure the materials exist to prevent. Named by stacking depth: primary sits on the backdrop, secondary sits over something already glass. See the glass page.",
    roles: [
      {
        token: "glass-primary",
        variable: "--bg-glass-primary",
        pointsAt: "glass 2 + blur-md + rim",
        use: "Glass sitting directly on the backdrop: chrome, nav, a region in glass. The most translucent, because the backdrop is the thing worth seeing through to.",
        status: "core",
      },
      {
        token: "glass-secondary",
        variable: "--bg-glass-secondary",
        pointsAt: "glass 4 + blur-lg + rim + shadow-sm",
        use: "Glass over something already glass: popovers, menus, a modal over a panel. Less translucent so it separates, and a real shadow because it is genuinely above.",
        status: "core",
      },
      {
        token: "bg-chrome-selected",
        variable: "--bg-chrome-selected",
        pointsAt: "neutral 1 light / neutral 5 dark",
        use: "The selected item inside chrome. Opaque rather than a glass step, because on glass elevation reads as less translucency, not a lighter colour.",
        status: "core",
      },
      {
        token: "glass-primary-interactive",
        variable: "--bg-glass-primary-hover",
        pointsAt: "glass 3 on hover",
        use: "Primary glass you can click: chrome buttons, topbar controls. The hover moves one step up the ramp and never changes blur — re-blurring a large surface every frame is expensive enough to feel.",
        status: "core",
      },
    ],
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
  { group: "emphasis", words: ["primary", "secondary", "tertiary", "default"] },
  { group: "state", words: ["hover", "selected", "disabled"] },
  { group: "material", words: ["glass"] },
  { group: "modifier", words: ["tint"] },
  { group: "identity", words: ["accent", "inverse"] },
  // Not a status vocabulary: `error` names one situation that needed a colour,
  // and `agent-running` names another. A tier vocabulary is what produced four
  // undesigned identities, so these stay concrete until a pattern repeats.
  { group: "situation", words: ["error", "agent-running"] },
  { group: "paired", words: ["on-accent", "on-inverse"] },
];

export const GRAMMAR = "<property>-<role>[-<modifier>][-<material>][-<state>]";

/** Fixed order, so there is only one correct spelling. */
export const GRAMMAR_EXAMPLES = {
  legal: ["bg-accent-tint-hover", "text-primary", "bg-chrome-selected"],
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

export const SPACE = [
  {
    step: 1,
    variable: "--space-1",
    value: "4px",
    use: "Tight optical separation inside a control.",
  },
  {
    step: 2,
    variable: "--space-2",
    value: "8px",
    use: "The panel gap and default gap between related items.",
  },
  {
    step: 3,
    variable: "--space-3",
    value: "12px",
    use: "The horizontal inset inside rows and compact controls.",
  },
  {
    step: 4,
    variable: "--space-4",
    value: "16px",
    use: "The workspace edge and ordinary content inset.",
  },
  {
    step: 5,
    variable: "--space-5",
    value: "20px",
    use: "The inset of a full panel header or reading surface.",
  },
  {
    step: 6,
    variable: "--space-6",
    value: "24px",
    use: "Separation between distinct content groups.",
  },
];

export const SPACE_ROLES = [
  {
    token: "space-workspace-inset",
    variable: "--space-workspace-inset",
    pointsAt: "space 4",
    use: "The distance between the window edge and its panels.",
  },
  {
    token: "space-panel-gap",
    variable: "--space-panel-gap",
    pointsAt: "space 2",
    use: "The seam between independently movable panels.",
  },
  {
    token: "space-panel-inset",
    variable: "--space-panel-inset",
    pointsAt: "space 5",
    use: "The horizontal inset inside a full panel.",
  },
  {
    token: "space-control-inset",
    variable: "--space-control-inset",
    pointsAt: "space 3",
    use: "The horizontal inset inside controls and navigation rows.",
  },
  {
    token: "space-row-gap",
    variable: "--space-row-gap",
    pointsAt: "space 2",
    use: "The gap between an icon, label, and trailing metadata in one row.",
  },
  {
    token: "space-section-gap",
    variable: "--space-section-gap",
    pointsAt: "space 4",
    use: "The gap between adjacent navigator sections. Larger than the 1px between rows inside one, because that difference is the only thing saying where a group ends.",
  },
];

export const RADII = [
  {
    token: "radius-row",
    variable: "--radius-row",
    value: "10px",
    use: "Dense navigator rows and compact selected regions.",
  },
  {
    token: "radius-control",
    variable: "--radius-control",
    value: "12px",
    use: "Inputs, buttons, tabs, and header actions.",
  },
  {
    token: "radius-panel",
    variable: "--radius-panel",
    value: "24px",
    use: "Every major workspace panel.",
  },
  {
    token: "radius-pill",
    variable: "--radius-pill",
    value: "round",
    use: "Pills, avatars, and fully circular controls.",
  },
];

export const MOTION = [
  {
    token: "duration-fast",
    variable: "--duration-fast",
    value: "120ms",
    use: "A hover affordance appearing or disappearing.",
  },
  {
    token: "duration-state",
    variable: "--duration-state",
    value: "150ms",
    use: "A colour or opacity state changing in place.",
  },
  {
    token: "duration-settle",
    variable: "--duration-settle",
    value: "220ms",
    use: "A tab indicator or released panel settling into place.",
  },
  {
    token: "easing-state",
    variable: "--easing-state",
    value: "ease",
    use: "Small state changes that do not move geometry.",
  },
  {
    token: "easing-settle",
    variable: "--easing-settle",
    value: "cubic-bezier(.645,.045,.355,1)",
    use: "Geometry settling after a deliberate selection or release.",
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
    name: "backdrop treatments and gradient-1…4, texture-dots",
    why: "Not colours in the ramp sense. A named backdrop treatment is a complete visual composition; gradient-1…4 pair one light scene and one dark scene for a stable appearance choice.",
  },
];
