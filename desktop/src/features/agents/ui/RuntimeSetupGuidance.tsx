import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { runtimeAvailabilityWarning } from "./runtimeAvailabilityWarning";

export function RuntimeSetupGuidance({
  configurationSatisfied = false,
  runtime,
}: {
  configurationSatisfied?: boolean;
  runtime: AcpRuntimeCatalogEntry | undefined;
}) {
  if (
    configurationSatisfied &&
    runtime?.runtimeReadiness === "configuration_required"
  ) {
    return null;
  }
  const warning = runtime ? runtimeAvailabilityWarning(runtime) : null;
  return warning ? (
    <p className="text-xs text-warning">
      {warning}
      {runtime?.runtimeReadiness === "configuration_required"
        ? null
        : " Visit Settings > Agents to set it up."}
    </p>
  ) : null;
}
