import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";

/**
 * Short setup detail for runtime setup surfaces.
 *
 * `installHint` remains the fallback/vendor guidance. For min-version failures
 * Buzz has structured version fields, so prefer the concise user-facing copy
 * and keep onboarding/settings aligned.
 */
export function runtimeSetupDetailText(
  runtime: AcpRuntimeCatalogEntry,
): string {
  if (runtime.availability !== "cli_outdated") {
    return runtime.installHint.trim();
  }

  if (runtime.cliVersion && runtime.minimumCliVersion) {
    return `${runtime.label} ${runtime.cliVersion} detected; requires ${runtime.minimumCliVersion} or newer.`;
  }

  if (runtime.minimumCliVersion) {
    return `${runtime.label} is outdated; requires ${runtime.minimumCliVersion} or newer.`;
  }

  return runtime.installHint.trim() || `${runtime.label} is outdated.`;
}
