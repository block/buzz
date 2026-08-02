import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";

/**
 * Availability warning sentence for the agent-definition dialog.
 * Returns null when the runtime is available (no warning to show).
 *
 * Non-empty install hints are appended so a not-installed or
 * adapter-missing runtime tells the user the actual next step (presets
 * always carry one; custom harnesses may leave it empty — guard on the
 * trimmed value to avoid a dangling "X is not installed. ").
 */
export function runtimeAvailabilityWarning(
  runtime: AcpRuntimeCatalogEntry,
): string | null {
  if (runtime.availability === "available") {
    return null;
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
