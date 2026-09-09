/** Role-based geometry and motion shared by components and documentation. */
export const FOUNDATION_GROUPS = [
  {
    name: "Controls",
    description: "Shared control dimensions and overlay stacking.",
    roles: [
      ["control-sm", "2rem", "Small actions."],
      ["control-md", "2.5rem", "Standard actions and inputs."],
      ["control-lg", "3.25rem", "Prominent actions."],
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
      "A four-pixel rhythm, expressed in rem so the entire interface follows zoom.",
    roles: [
      ["space-control-gap", "0.5rem", "Space between a label and its icon."],
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
