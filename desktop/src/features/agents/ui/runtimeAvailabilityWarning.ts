import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";

/**
 * Setup warning sentence for an unavailable or not-yet-ready catalog runtime.
 * Install and login hints come directly from the catalog; no runtime identity is
 * inferred in the UI.
 */
export function runtimeAvailabilityWarning(
  runtime: AcpRuntimeCatalogEntry,
): string | null {
  if (runtime.availability === "available") {
    const loginHint = runtime.loginHint?.trim();
    switch (runtime.runtimeReadiness) {
      case "ready":
        return null;
      case "configuration_required":
        return `${runtime.label} needs its required connection fields configured.`;
      case "authentication_required":
        return loginHint
          ? `${runtime.label} requires sign-in. ${loginHint}`
          : `${runtime.label} requires sign-in.`;
      case "model_unavailable":
        return `${runtime.label} needs a model configured before it can run.`;
      case "unknown":
        return `${runtime.label}'s setup could not be verified.`;
    }
  }

  const hint = runtime.installHint.trim();
  const withHint = (base: string) => (hint ? `${base} ${hint}` : base);
  switch (runtime.availability) {
    case "adapter_missing":
      return withHint(
        `${runtime.label} CLI is installed but the ACP adapter is missing.`,
      );
    case "adapter_outdated":
      return `${runtime.label} ACP adapter is outdated — reinstall to continue.`;
    default:
      return runtime.requiresExternalCli
        ? withHint(`${runtime.label} CLI is missing.`)
        : withHint(`${runtime.label} is not installed.`);
  }
}
