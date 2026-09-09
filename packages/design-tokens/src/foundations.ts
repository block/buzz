/** Role-based geometry and motion shared by components and documentation. */
export const FOUNDATION_GROUPS = [
  {
    name: "Controls",
    description: "Shared control dimensions and overlay stacking.",
    roles: [
      ["control-sm", "2rem", "Small actions."],
      ["control-md", "2.5rem", "Standard actions and inputs."],
      ["control-lg", "3.25rem", "Prominent actions."],
      ["control-calendar", "2.25rem", "Calendar day and navigation targets."],
      [
        "control-textarea-min",
        "7rem",
        "Minimum height for longer text fields.",
      ],
      [
        "control-multiline-min",
        "5rem",
        "Minimum writing area for a multiline prompt.",
      ],
      [
        "control-multiline-max",
        "16rem",
        "Maximum growing writing area before scrolling.",
      ],
      [
        "size-scrollbar",
        "0.375rem",
        "Thin custom scrollbars and native scrollbar fallback.",
      ],
      [
        "size-grip-length",
        "1rem",
        "Visible resize grip length, independent of its hit area.",
      ],
      ["size-grip-thickness", "0.25rem", "Visible resize grip thickness."],
      [
        "size-resize-target",
        "1.5rem",
        "Resize hit area preserved at either density.",
      ],
      ["space-action-inset", "1.25rem", "Standard button horizontal padding."],
      ["space-tooltip-block", "0.125rem", "Compact tooltip vertical padding."],
      ["layer-dialog", "80", "Modal surfaces."],
      ["layer-floating", "100", "Menus and tooltips above their trigger."],
      ["layer-toast", "120", "Transient status announcements."],
      ["overlay-scrim", "rgb(0 0 0 / 0.5)", "A modal backdrop."],
    ],
  },
  {
    name: "Spacing",
    description:
      "A four-pixel layout rhythm in Normal and a three-pixel rhythm in Compact, with named component insets. Rem units keep both responsive to zoom.",
    roles: [
      [
        "spacing",
        "0.25rem",
        "Base layout rhythm for Tailwind spacing utilities.",
      ],
      ["space-control-gap", "0.5rem", "Space between a label and its icon."],
      ["space-chip-block", "0.375rem", "Chip and badge vertical inset."],
      ["space-chip-inline", "0.625rem", "Chip and badge horizontal inset."],
      ["space-catalog-gap", "3rem", "Space between catalog rows."],
      ["size-catalog-preview", "10rem", "Minimum catalog preview height."],
      ["space-control-inset", "0.75rem", "Compact control padding."],
      [
        "space-heading-gap",
        "1rem",
        "Space between a heading and its description.",
      ],
      ["space-section-gap", "1.5rem", "Space between related groups."],
      ["space-panel-inset", "1.5rem", "Padding inside a panel."],
      ["space-page-inset", "2rem", "Outer reading gutter."],
    ],
  },
  {
    name: "Radius",
    description:
      "Corners describe the surface: controls, notices, menus, containers, overlays, and sheets.",
    roles: [
      ["radius-control", "0.5rem", "Inputs and checkable controls."],
      ["radius-notice", "0.75rem", "Inline notices and tooltips."],
      ["radius-menu", "1rem", "Menus and popovers."],
      ["radius-container", "1.5rem", "Cards and grouped content."],
      ["radius-overlay", "2rem", "Dialogs."],
      ["radius-sheet", "2.5rem", "Large sheets."],
      ["radius-pill", "9999px", "Buttons and compact identity chips."],
    ],
  },
  {
    name: "Motion",
    description:
      "State changes are short. Entering surfaces settle gently. Reduced motion removes movement.",
    roles: [
      ["duration-state", "120ms", "Hover and focus changes."],
      ["duration-enter", "180ms", "Opening a surface."],
      ["duration-exit", "120ms", "Closing a surface."],
      [
        "ease-standard",
        "cubic-bezier(0.2, 0, 0, 1)",
        "A quick response with a quiet finish.",
      ],
      ["press-scale", "0.97", "Subtle feedback while a button is held."],
    ],
  },
] as const;

/** Proposed compact density. Unlisted roles retain Normal values. Text, icons,
 * motion, and fine interaction geometry are deliberately independent of density. */
export const COMPACT_FOUNDATIONS: Partial<
  Record<(typeof FOUNDATION_GROUPS)[number]["roles"][number][0], string>
> = {
  "control-sm": "1.75rem",
  "control-md": "2rem",
  "control-lg": "2.5rem",
  "control-calendar": "1.75rem",
  "control-textarea-min": "5rem",
  "control-multiline-min": "4rem",
  "space-action-inset": "0.75rem",
  spacing: "0.1875rem",
  "space-control-gap": "0.25rem",
  "space-control-inset": "0.5rem",
  "space-heading-gap": "0.5rem",
  "space-section-gap": "1rem",
  "space-panel-inset": "1rem",
  "space-page-inset": "1.5rem",
  "space-chip-block": "0.25rem",
  "space-chip-inline": "0.5rem",
  "space-catalog-gap": "2rem",
  "size-catalog-preview": "7rem",
  "radius-control": "0.375rem",
  "radius-notice": "0.5rem",
  "radius-menu": "0.75rem",
  "radius-container": "1rem",
  "radius-overlay": "1.5rem",
  "radius-sheet": "2rem",
};
