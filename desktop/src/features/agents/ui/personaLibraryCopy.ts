export const personaLibraryCopy = {
  title: "My agents",
  description:
    "The agents you have chosen for this app. Use them to create teams and launch agents.",
  chooseFromCatalog: "Choose from catalog",
  createNew: "New agent",
  import: "Import snapshot",
  emptyTitle: "No agents yet",
  emptyDescription:
    "Choose one from Agent Catalog, create your own, or import one to get started.",
  emptyImportHint:
    "Or drop an .agent.json or .agent.png snapshot here to import.",
} as const;

export const personaCatalogCopy = {
  title: "Agent Catalog",
  description: "Browse agents shared to this relay.",
  dialogTitle: "Agent Catalog",
  dialogDescription: "Browse agents shared to this relay.",
  emptyTitle: "You're all set",
  emptyDescription: "Everything in Agent Catalog is already in My Agents.",
  emptyCatalogDescription: "Shared agents will appear here.",
  emptyCatalogTitle: "No agents are being shared",
  detailsAction: "View details",
  selectAction: "Choose",
  deselectAction: "Deselect",
  selectedState: "Selected",
  availableState: "Available",
  detailSelectedTitle: "Selected for My Agents",
  detailSelectedDescription:
    "Turn this off to remove the agent from teams and agent creation in this app.",
  detailAvailableTitle: "Available in Agent Catalog",
  detailAvailableDescription:
    "Turn this on to make the agent available for teams and agent creation.",
  useAction: "Add agent",
  addedAction: "Added to My Agents",
  teamEmptyState:
    "No agents in My Agents yet. Create one or choose one from Agent Catalog first.",
} as const;

export function getPersonaCatalogSelectionActionCopy(isActive: boolean) {
  return isActive
    ? personaCatalogCopy.deselectAction
    : personaCatalogCopy.selectAction;
}

export function getPersonaCatalogSelectionAriaLabel(
  displayName: string,
  isActive: boolean,
) {
  return `${isActive ? "Deselect" : "Select"} ${displayName} in My Agents`;
}

export function getPersonaCatalogDetailSelectionCopy(isActive: boolean) {
  return isActive
    ? {
        title: personaCatalogCopy.detailSelectedTitle,
        description: personaCatalogCopy.detailSelectedDescription,
      }
    : {
        title: personaCatalogCopy.detailAvailableTitle,
        description: personaCatalogCopy.detailAvailableDescription,
      };
}
