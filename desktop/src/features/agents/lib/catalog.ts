import type { AgentPersona } from "@/shared/api/types";

export type CatalogSelectionState = {
  catalogPersonas: AgentPersona[];
  selectedCatalogPersonas: AgentPersona[];
  unselectedCatalogPersonas: AgentPersona[];
};

export type PersonaLibraryState = {
  catalogPersonas: AgentPersona[];
  libraryPersonas: AgentPersona[];
  personaLabelsById: Record<string, string>;
};

export function isPersonaActive(persona: AgentPersona) {
  return persona.isActive;
}

export function getActivePersonas(personas: readonly AgentPersona[]) {
  return personas.filter(isPersonaActive);
}

export function getLibraryPersonas(personas: readonly AgentPersona[]) {
  return getActivePersonas(personas);
}

export function isPersonaVisibleInCatalog(
  persona: AgentPersona,
  sharedCatalogPersonaIds: ReadonlySet<string> = new Set(),
) {
  return persona.isBuiltIn || sharedCatalogPersonaIds.has(persona.id);
}

export function getCatalogPersonas(
  personas: readonly AgentPersona[],
  sharedCatalogPersonaIds: ReadonlySet<string> = new Set(),
) {
  return personas
    .filter((persona) =>
      isPersonaVisibleInCatalog(persona, sharedCatalogPersonaIds),
    )
    .sort((left, right) => left.displayName.localeCompare(right.displayName));
}

export function isCatalogPersonaSelected(persona: AgentPersona) {
  return persona.isActive;
}

export function getCatalogSelectionState(
  personas: readonly AgentPersona[],
  sharedCatalogPersonaIds: ReadonlySet<string> = new Set(),
): CatalogSelectionState {
  const catalogPersonas = getCatalogPersonas(personas, sharedCatalogPersonaIds);

  return {
    catalogPersonas,
    selectedCatalogPersonas: catalogPersonas.filter(isCatalogPersonaSelected),
    unselectedCatalogPersonas: catalogPersonas.filter(
      (persona) => !isCatalogPersonaSelected(persona),
    ),
  };
}

export function getPersonaLabelsById(personas: readonly AgentPersona[]) {
  return Object.fromEntries(
    personas.map((persona) => [persona.id, persona.displayName]),
  );
}

export function getPersonaLibraryState(
  personas: readonly AgentPersona[],
  sharedCatalogPersonaIds: ReadonlySet<string> = new Set(),
): PersonaLibraryState {
  const libraryPersonas = getLibraryPersonas(personas);
  const { catalogPersonas } = getCatalogSelectionState(
    personas,
    sharedCatalogPersonaIds,
  );

  return {
    catalogPersonas,
    libraryPersonas,
    personaLabelsById: getPersonaLabelsById(personas),
  };
}
