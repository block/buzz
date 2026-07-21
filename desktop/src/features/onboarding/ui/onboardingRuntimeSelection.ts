import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";

export const ONBOARDING_RUNTIME_ORDER = ["claude", "codex"];

const KNOWN_ONBOARDING_RUNTIME_IDS = new Set<string>(ONBOARDING_RUNTIME_ORDER);

export function runtimeIsOnboardingChoice(runtimeId: string) {
  return KNOWN_ONBOARDING_RUNTIME_IDS.has(runtimeId);
}

export function runtimeUsesDefaultModelConfig(runtimeId: string) {
  return runtimeId === "buzz-agent" || runtimeId === "goose";
}

export function getDefaultModelConfigRuntimeId(runtimeIds: readonly string[]) {
  return (
    runtimeIds.find((runtimeId) => runtimeId === "buzz-agent") ??
    runtimeIds.find((runtimeId) => runtimeId === "goose") ??
    null
  );
}

export function getPreferredRuntimeIdForSelection(
  runtimeIds: readonly string[],
) {
  const selectedRuntimeIds = new Set(runtimeIds);
  return (
    ONBOARDING_RUNTIME_ORDER.find((runtimeId) =>
      selectedRuntimeIds.has(runtimeId),
    ) ??
    runtimeIds[0] ??
    null
  );
}

export function runtimeSelectionNeedsDefaultModelConfig(
  runtimeIds: readonly string[],
) {
  return runtimeIds.some(runtimeUsesDefaultModelConfig);
}

export function runtimeSelectionNeedsDefaultsStep(
  runtimeIds: readonly string[],
) {
  return runtimeIds.length > 0;
}

export function runtimeCanBeSelected(runtime: AcpRuntimeCatalogEntry) {
  return runtimeIsOnboardingChoice(runtime.id);
}

export function runtimeCanAdvanceOnboarding(runtime: AcpRuntimeCatalogEntry) {
  return (
    runtime.availability === "available" &&
    (runtime.authStatus.status === "logged_in" ||
      runtime.authStatus.status === "not_applicable")
  );
}
