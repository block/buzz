export type AgentConfigDisclosure =
  | "full"
  | "onboarding-essential"
  | "progressive-defaults";

// Canonical behaviors (PR 2 flag cleanup). Onboarding's values won every
// former per-surface prop and are now the only behavior.
export const CANONICAL_CONFIG_BEHAVIORS = {
  autoSelectModelOnProviderChange: true,
  disableModelSelectDuringDiscovery: false,
  preserveCredentialEnvVarsOnProviderChange: true,
  requireProviderForModelAndEffort: true,
} as const;

/** Disclosure preset to the visibility decisions it owns. */
export function resolveDisclosure(disclosure: AgentConfigDisclosure) {
  const full = disclosure !== "onboarding-essential";
  return {
    showAdvancedFields: full,
    showCustomModelOption: full,
    showCustomProviderOption: full,
    showDescriptions: full,
    showEffortField: true,
    showProviderPlaceholderOption: full,
    showRequiredIndicators: full,
    showUnavailableEffortOptions: full,
  } as const;
}

export function shouldRevealDependentConfigFields({
  disclosure,
  providerFieldVisible,
  providerValue,
}: {
  disclosure: AgentConfigDisclosure;
  providerFieldVisible: boolean;
  providerValue: string;
}): boolean {
  return (
    disclosure !== "progressive-defaults" ||
    !providerFieldVisible ||
    providerValue.trim().length > 0
  );
}

/** Discovery warnings bypass reduced disclosure so failures remain visible. */
export function shouldShowModelStatusMessage(
  showDescriptions: boolean,
  status: { message: string; tone: string } | null,
): boolean {
  return showDescriptions || status !== null;
}

/** Optional-model controls hide only after a confirmed successful empty result. */
export function shouldRenderModelControl({
  discoveredModelOptions,
  modelDiscoveryLoading,
  modelDiscoverySuccessfulEmpty,
  modelIsOptional,
  showCustomModelOption,
}: {
  discoveredModelOptions: readonly { id: string }[] | null;
  modelDiscoveryLoading: boolean;
  modelDiscoverySuccessfulEmpty: boolean;
  modelIsOptional: boolean;
  showCustomModelOption: boolean;
}): boolean {
  if (!modelIsOptional) return true;
  if (modelDiscoveryLoading) return false;
  const hasExplicitModel = (discoveredModelOptions ?? []).some(
    (option) => option.id.trim().length > 0,
  );
  if (hasExplicitModel) return true;
  if (showCustomModelOption) return true;
  return !modelDiscoverySuccessfulEmpty;
}
