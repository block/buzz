import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";

export const ONBOARDING_RUNTIME_ORDER = ["claude", "codex"];

const VISIBLE_ONBOARDING_RUNTIME_IDS = new Set<string>(
  ONBOARDING_RUNTIME_ORDER,
);

export function runtimeIsVisibleInOnboarding(runtimeId: string) {
  return VISIBLE_ONBOARDING_RUNTIME_IDS.has(runtimeId);
}

export function runtimeIsReadyForOnboarding(runtime: AcpRuntimeCatalogEntry) {
  return (
    runtime.availability === "available" &&
    (runtime.authStatus.status === "logged_in" ||
      runtime.authStatus.status === "not_applicable")
  );
}

export function getVisibleOnboardingRuntimes(
  runtimes: readonly AcpRuntimeCatalogEntry[],
) {
  return runtimes
    .filter((runtime) => runtimeIsVisibleInOnboarding(runtime.id))
    .sort(
      (left, right) =>
        ONBOARDING_RUNTIME_ORDER.indexOf(left.id) -
        ONBOARDING_RUNTIME_ORDER.indexOf(right.id),
    );
}

export function getReadyOnboardingRuntimes(
  runtimes: readonly AcpRuntimeCatalogEntry[],
) {
  return getVisibleOnboardingRuntimes(runtimes).filter(
    runtimeIsReadyForOnboarding,
  );
}
