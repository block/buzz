import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";

export function runtimeIsVisibleInOnboarding(runtime: AcpRuntimeCatalogEntry) {
  return runtime.onboardingVisible;
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
    .filter(runtimeIsVisibleInOnboarding)
    .sort(
      (left, right) =>
        left.sortPriority - right.sortPriority ||
        (left.displayLabel || left.label || left.id).localeCompare(
          right.displayLabel || right.label || right.id,
        ),
    );
}

export function getReadyOnboardingRuntimes(
  runtimes: readonly AcpRuntimeCatalogEntry[],
) {
  return getVisibleOnboardingRuntimes(runtimes).filter(
    runtimeIsReadyForOnboarding,
  );
}
