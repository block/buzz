export type PrimaryMenuItemId =
  | "inbox"
  | "pulse"
  | "projects"
  | "browsers"
  | "agents"
  | "bots"
  | "workflows";

export type PrimaryMenuItem = {
  id: PrimaryMenuItemId;
  label: string;
  testId: string;
};

/**
 * Primary-menu order. Feature-gated items stay in this list so placement
 * tests do not depend on which preview flags are on.
 */
export const PRIMARY_MENU_ITEMS: readonly PrimaryMenuItem[] = [
  { id: "inbox", label: "Inbox", testId: "open-inbox-view" },
  { id: "pulse", label: "Pulse", testId: "open-pulse-view" },
  { id: "projects", label: "Projects", testId: "open-projects-view" },
  { id: "agents", label: "Agents", testId: "open-agents-view" },
  { id: "bots", label: "Bots", testId: "open-bots-view" },
  { id: "browsers", label: "Browsers", testId: "open-browsers-view" },
  { id: "workflows", label: "Workflows", testId: "open-workflows-view" },
];

export function primaryMenuLabels(): string[] {
  return PRIMARY_MENU_ITEMS.map((item) => item.label);
}

export function primaryMenuItemAfter(id: PrimaryMenuItemId): PrimaryMenuItem {
  const index = PRIMARY_MENU_ITEMS.findIndex((item) => item.id === id);
  const next = PRIMARY_MENU_ITEMS[index + 1];
  if (!next) {
    throw new Error(`No primary-menu item after ${id}`);
  }
  return next;
}
