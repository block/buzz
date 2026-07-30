import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";

function cliOutdatedVersionHint(runtime: AcpRuntimeCatalogEntry): string {
  if (runtime.cliVersion && runtime.minimumCliVersion) {
    return `Detected ${runtime.label} ${runtime.cliVersion}. Buzz requires ${runtime.minimumCliVersion} or newer.`;
  }
  if (runtime.minimumCliVersion) {
    return `Buzz could not verify the ${runtime.label} version. Buzz requires ${runtime.minimumCliVersion} or newer.`;
  }
  return "";
}

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
    case "cli_outdated": {
      const versionHint = hint || cliOutdatedVersionHint(runtime);
      return versionHint
        ? `${runtime.label} is outdated. ${versionHint}`
        : `${runtime.label} is outdated.`;
    }
    default:
      return runtime.requiresExternalCli
        ? withHint(`${runtime.label} CLI is missing.`)
        : withHint(`${runtime.label} is not installed.`);
  }
}
