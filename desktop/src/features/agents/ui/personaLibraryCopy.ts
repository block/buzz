import { i18n } from "@/i18n";

/**
 * Copy for the "My agents" / "Agent Catalog" library surfaces.
 *
 * Resolved through `i18n.t` at call time (these are plain functions, not hooks),
 * so every caller reads the strings while rendering rather than at import.
 */
export function personaLibraryCopy() {
  return {
    title: i18n.t("agents.persona-library.title"),
    description: i18n.t("agents.persona-library.description"),
    chooseFromCatalog: i18n.t("agents.persona-library.choose-from-catalog"),
    createNew: i18n.t("agents.persona-library.create-new"),
    import: i18n.t("agents.persona-library.import"),
    emptyTitle: i18n.t("agents.persona-library.empty-title"),
    emptyDescription: i18n.t("agents.persona-library.empty-description"),
    emptyImportHint: i18n.t("agents.persona-library.empty-import-hint"),
  };
}

export function personaCatalogCopy() {
  return {
    title: i18n.t("agents.persona-catalog.title"),
    description: i18n.t("agents.persona-catalog.description"),
    dialogTitle: i18n.t("agents.persona-catalog.add-agent"),
    dialogDescription: i18n.t("agents.persona-catalog.description"),
    emptyTitle: i18n.t("agents.persona-catalog.empty-title"),
    emptyDescription: i18n.t("agents.persona-catalog.empty-description"),
    emptyCatalogDescription: i18n.t(
      "agents.persona-catalog.empty-catalog-description",
    ),
    emptyCatalogTitle: i18n.t("agents.persona-catalog.empty-catalog-title"),
    detailsAction: i18n.t("agents.persona-catalog.details-action"),
    selectAction: i18n.t("agents.persona-catalog.select-action"),
    deselectAction: i18n.t("agents.persona-catalog.deselect-action"),
    selectedState: i18n.t("agents.persona-catalog.selected-state"),
    availableState: i18n.t("agents.persona-catalog.available-state"),
    detailSelectedTitle: i18n.t("agents.persona-catalog.detail-selected-title"),
    detailSelectedDescription: i18n.t(
      "agents.persona-catalog.detail-selected-description",
    ),
    detailAvailableTitle: i18n.t(
      "agents.persona-catalog.detail-available-title",
    ),
    detailAvailableDescription: i18n.t(
      "agents.persona-catalog.detail-available-description",
    ),
    useAction: i18n.t("agents.persona-catalog.add-agent"),
    addedAction: i18n.t("agents.persona-catalog.added-action"),
    teamEmptyState: i18n.t("agents.persona-catalog.team-empty-state"),
  };
}

export function getPersonaCatalogSelectionActionCopy(isActive: boolean) {
  const copy = personaCatalogCopy();
  return isActive ? copy.deselectAction : copy.selectAction;
}

export function getPersonaCatalogSelectionAriaLabel(
  displayName: string,
  isActive: boolean,
) {
  return isActive
    ? i18n.t("agents.persona-catalog.deselect-aria", { displayName })
    : i18n.t("agents.persona-catalog.select-aria", { displayName });
}

export function getPersonaCatalogDetailSelectionCopy(isActive: boolean) {
  const copy = personaCatalogCopy();
  return isActive
    ? {
        title: copy.detailSelectedTitle,
        description: copy.detailSelectedDescription,
      }
    : {
        title: copy.detailAvailableTitle,
        description: copy.detailAvailableDescription,
      };
}
