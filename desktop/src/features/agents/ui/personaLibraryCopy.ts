export const personaLibraryCopy = {
  title: "My agents",
  description:
    "The agents you have chosen for this app. Use them to create teams and launch agents.",
  chooseFromCatalog: "Choose from catalog",
  createNew: "New agent",
  customAgent: "Custom agent",
  import: "Import",
  emptyTitle: "No agents yet",
  emptyDescription:
    "Choose one from Agent Catalog, create your own, or import one to get started.",
  emptyImportHint:
    "Or drop a .persona.md, .persona.json, .persona.png, or .zip file here to import.",
} as const;

export const personaCatalogCopy = {
  title: "Agent Catalog",
  description: "Browse built-in agents and add them to My Agents.",
  dialogTitle: "Agent Catalog",
  dialogDescription: "Browse built-in agents and add them to My Agents.",
  emptyTitle: "You're all set",
  emptyDescription: "Everything in Agent Catalog is already in My Agents.",
  emptyCatalogDescription:
    "New agents will show up here when the app ships more options.",
  emptyCatalogTitle: "No agents in the catalog yet",
  detailsAction: "View details",
  selectAction: "Choose",
  deselectAction: "Selected",
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
