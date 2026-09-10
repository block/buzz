import type { GrammarSection } from "./types";

export const foundationGrammar: GrammarSection[] = [
  {
    id: "role-resolution",
    title: "Names, values, and usage",
    sources: [
      "docs/role-join.md",
      "docs/color.roles.md",
      "foundations/roles/role-usage.json",
    ],
    rows: [
      [
        "Resolution",
        "Choose a semantic role, then resolve its primitive and platform binding.",
        "A similarly named platform token may have a different value.",
      ],
      [
        "Role join",
        "Connect the token’s dot-notation key to its usage rule; slash names can be input aliases.",
        "Family-level guidance is less specific than an authored member rule.",
      ],
      [
        "Usage contract",
        "Consult when, why, and do_nots together.",
        "A legal token can still be wrong for the situation.",
      ],
      [
        "Missing guidance",
        "Report that no rule has been authored when resolution has no exact usage match.",
        "Do not manufacture a rule or claim complete coverage.",
      ],
      [
        "Color naming",
        "Primary / secondary / tertiary is the chosen ordinal vocabulary.",
        "The reference snapshot still contains older standard / subtle / muted keys; match their meaning.",
      ],
      [
        "New roles",
        "Add a role only for a distinct intent that existing roles cannot express.",
        "A new size or visual preference alone is not a new semantic role.",
      ],
    ],
  },
  {
    id: "color",
    title: "Color and emphasis",
    sources: [
      "docs/color.roles.md",
      "docs/color.resolution.draft.json",
      "foundations/roles/role-usage.json",
    ],
    rows: [
      [
        "Channel",
        "Choose text, icon, surface, or border according to what receives the color.",
        "An unassigned channel is a deliberate limit, not permission to invent a fill.",
      ],
      [
        "Primary text",
        "Titles, key values, names, body content, and primary labels.",
        "Use quieter roles for supporting metadata.",
      ],
      [
        "Secondary text",
        "Supporting explanations and lower-priority content.",
        "Quiet content remains readable and available.",
      ],
      [
        "Tertiary text",
        "Timestamps, placeholders, annotations, and other scaffolding.",
        "Do not substitute disabled styling for lower emphasis.",
      ],
      [
        "Inverse",
        "Pair inverse content with an inverted surface.",
        "Elevation alone does not imply inversion; both colors must work in either theme.",
      ],
      [
        "Icons",
        "Match the corresponding content role.",
        "Do not introduce a competing emphasis level beside its label.",
      ],
      [
        "Surfaces",
        "Distinguish page, inset/group, selected/emphasis, card, and floating-overlay jobs.",
        "Platform-specific fills may resolve differently; follow the selected platform.",
      ],
      [
        "Borders",
        "Subtle separates; standard outlines a control; prominent emphasizes an edge; focus marks keyboard focus.",
        "Do not use a high-emphasis outline for an ordinary divider.",
      ],
      [
        "Status",
        "Use semantic color for a real error, caution, or confirmation.",
        "Product-specific channel limits apply; do not invent a success background.",
      ],
      [
        "Brand",
        "Use only where an identity treatment is explicitly assigned.",
        "Brand is not a general-purpose accent for actions, status, or decoration.",
      ],
      [
        "Trend",
        "Use signs, arrows, and words for ordinary up/down/flat changes.",
        "Direction alone does not warrant success or error color.",
      ],
      [
        "Chart focus",
        "Keep context marks neutral and distinguish the focal item with an assigned product color.",
        "The focal encoding is separate from trend or status semantics.",
      ],
    ],
  },
  {
    id: "states",
    title: "Interaction states",
    sources: ["docs/color.resolution.draft.json", "Design.md"],
    rows: [
      [
        "Rest",
        "Use the role’s resolved default value.",
        "State changes derive from that resting role.",
      ],
      [
        "Hover",
        "Move the surface one step toward stronger contrast.",
        "Text and icons stay stable; hover is a pointer affordance.",
      ],
      [
        "Pressed",
        "The reference color model moves the surface two contrast steps.",
        "Follow the target platform’s active-state behavior.",
      ],
      [
        "Selected",
        "Use a persistent neutral contrast change.",
        "A narrow product-specific colored selection exception is not a global accent rule.",
      ],
      [
        "Focus",
        "Show the keyboard focus ring without changing the fill.",
        "Keep focus distinct from hover and selection.",
      ],
      [
        "Disabled",
        "Use disabled tokens and actual unavailable-control semantics.",
        "Do not use disabled to mean merely secondary, or rely only on color.",
      ],
      [
        "Loading",
        "Represent the pending action in the control’s state contract.",
        "Preserve its accessible name and avoid implying completion.",
      ],
    ],
  },
  {
    id: "geometry",
    title: "Spacing, shape, and grouping",
    sources: [
      "Design.md",
      "docs/shape.resolution.draft.json",
      "foundations/roles/role-usage.json",
      "foundations/elevation/README.md",
      "foundations/motion/README.md",
    ],
    rows: [
      [
        "Spacing",
        "BlockUI’s structural reference starts from an eight-unit rhythm.",
        "Observed spacing examples are not universal prescriptions; specific surface rules take precedence.",
      ],
      [
        "Grouping",
        "Try whitespace first, a divider next, and a container when stronger separation is needed.",
        "Do not frame every related pair of elements.",
      ],
      [
        "control",
        "Input-scale fields and small interactive elements.",
        "Avoid content-container corners on small controls.",
      ],
      [
        "menu",
        "Lightweight menus and transient popovers.",
        "A primary modal or owned content surface uses a container role.",
      ],
      [
        "container",
        "Cards and primary content or modal containers.",
        "The applet shell has its own more specific geometry.",
      ],
      [
        "overlay / sheet",
        "Large edge-attached overlays and bottom sheets.",
        "The source documents platform differences rather than one universal radius.",
      ],
      [
        "pill",
        "Fully rounded shapes wider than they are tall.",
        "Resolve full rounding as a shape, not an arbitrary corner value.",
      ],
      [
        "circular",
        "Square avatars, round icon controls, and other equal-sided shapes.",
        "Do not force variable-width text into a circle.",
      ],
      [
        "Elevation / motion",
        "Consult the foundation handoffs for platform-specific behavior and open work.",
        "Elevation is a proposed lane in this snapshot; the motion comparison is not one settled universal grammar.",
      ],
    ],
  },
  {
    id: "components",
    title: "Component contracts",
    sources: ["Design.md"],
    rows: [
      [
        "Base component",
        "A product-independent job with reusable content, state, interaction, or accessibility behavior.",
        "Complexity does not disqualify a Table or Calendar from being a base component.",
      ],
      [
        "Product composition",
        "A domain-specific workflow assembled from base components.",
        "Repeated use alone does not make its product policy a base contract.",
      ],
      [
        "Button axes",
        "Choose emphasis/tone and size independently.",
        "A screen-level CTA is a layout role, not a special size token.",
      ],
      [
        "Action hierarchy",
        "Usually one primary action per task context; supporting actions use secondary treatment.",
        "Reserve ghost treatment for genuinely contextual or lower-priority actions.",
      ],
      [
        "Button anatomy",
        "A text label with optional leading and trailing content.",
        "Icon-only controls need an accessible name.",
      ],
      [
        "Ownership",
        "Button owns the trigger; menu owns popup items, focus, keyboard navigation, and dismissal.",
        "Select and Combobox own form-value selection; links own navigation.",
      ],
      [
        "Fields",
        "Associate a label, help or validation, and the input.",
        "Do not remove functional help when simplifying surrounding documentation.",
      ],
    ],
  },
];
