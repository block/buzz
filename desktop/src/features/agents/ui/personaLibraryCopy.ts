// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
export const personaLibraryCopy = {
  get title() {
    return uiT("My agents");
  },
  get description() {
    return uiT(
      "The agents you have chosen for this app. Use them to create teams and launch agents.",
    );
  },
  get chooseFromCatalog() {
    return uiT("Choose from catalog");
  },
  get createNew() {
    return uiT("New agent");
  },
  get import() {
    return uiT("Import snapshot");
  },
  get emptyTitle() {
    return uiT("No agents yet");
  },
  get emptyDescription() {
    return uiT(
      "Choose one from Agent Catalog, create your own, or import one to get started.",
    );
  },
  get emptyImportHint() {
    return uiT("Or drop an .agent.json or .agent.png snapshot here to import.");
  },
} as const;

export const personaCatalogCopy = {
  get title() {
    return uiT("Agent Catalog");
  },
  get description() {
    return uiT("Browse agents shared to this relay.");
  },
  get dialogTitle() {
    return uiT("Add agent");
  },
  get dialogDescription() {
    return uiT("Browse agents shared to this relay.");
  },
  get emptyTitle() {
    return uiT("You're all set");
  },
  get emptyDescription() {
    return uiT("Everything in Agent Catalog is already in My Agents.");
  },
  get emptyCatalogDescription() {
    return uiT("Shared agents will appear here.");
  },
  get emptyCatalogTitle() {
    return uiT("No agents are being shared");
  },
  get detailsAction() {
    return uiT("View details");
  },
  get selectAction() {
    return uiT("Choose");
  },
  get deselectAction() {
    return uiT("Deselect");
  },
  get selectedState() {
    return uiT("Selected");
  },
  get availableState() {
    return uiT("Available");
  },
  get detailSelectedTitle() {
    return uiT("Selected for My Agents");
  },
  get detailSelectedDescription() {
    return uiT(
      "Turn this off to remove the agent from teams and agent creation in this app.",
    );
  },
  get detailAvailableTitle() {
    return uiT("Available in Agent Catalog");
  },
  get detailAvailableDescription() {
    return uiT(
      "Turn this on to make the agent available for teams and agent creation.",
    );
  },
  get useAction() {
    return uiT("Add agent");
  },
  get addedAction() {
    return uiT("Added to My Agents");
  },
  get teamEmptyState() {
    return uiT(
      "No agents in My Agents yet. Create one or choose one from Agent Catalog first.",
    );
  },
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
  return uiT("{0} {1} in My Agents", {
    p0: isActive ? "Deselect" : "Select",
    p1: displayName,
  });
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
