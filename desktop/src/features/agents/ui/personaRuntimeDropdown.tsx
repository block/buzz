import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import {
  AUTO_MODEL_DROPDOWN_VALUE,
  formatRuntimeOptionLabel,
  NO_RUNTIME_DROPDOWN_VALUE,
  type PersonaDropdownOption,
  sortPersonaRuntimes,
} from "./agentConfigOptions";
import { modelDropdownOptions as buildModelDropdownOptions } from "./relayMeshModelPicker";

/**
 * Harness options for the definition dialog.
 *
 * Create mode has no blank row — a new definition must pick a harness — while
 * edit mode keeps "No preference" so an existing definition can fall back to
 * the app default. A currently-set harness the catalog does not know about is
 * appended so editing an unrelated field cannot silently drop it.
 */
export function buildPersonaRuntimeDropdown({
  defaultRuntime,
  isCreateMode,
  runtime,
  runtimes,
  runtimesLoading,
}: {
  defaultRuntime: AcpRuntimeCatalogEntry | null;
  isCreateMode: boolean;
  runtime: string;
  runtimes: AcpRuntimeCatalogEntry[];
  runtimesLoading: boolean;
}): { blankLabel: string; options: PersonaDropdownOption[] } {
  const blankLabel = runtimesLoading
    ? "Loading harnesses..."
    : isCreateMode
      ? "Choose a harness"
      : "No preference (use app default)";

  const options: PersonaDropdownOption[] = [
    ...(!isCreateMode
      ? [{ label: blankLabel, value: NO_RUNTIME_DROPDOWN_VALUE }]
      : []),
    ...sortPersonaRuntimes(runtimes).map((candidate) => ({
      disabled:
        isCreateMode &&
        defaultRuntime !== null &&
        candidate.availability !== "available",
      label: `${formatRuntimeOptionLabel(candidate)}${
        isCreateMode && candidate.id === defaultRuntime?.id ? " (default)" : ""
      }`,
      value: candidate.id,
    })),
  ];

  if (
    runtime.trim().length > 0 &&
    !options.some((option) => option.value === runtime)
  ) {
    options.push({
      label: `${runtime.trim()} (current)`,
      value: runtime.trim(),
    });
  }

  return { blankLabel, options };
}

/** Why the selected harness cannot run, and where to fix it. */
export function PersonaRuntimeWarning({
  runtime,
}: {
  runtime: AcpRuntimeCatalogEntry | undefined;
}) {
  if (!runtime || runtime.availability === "available") return null;
  return (
    <p className="text-xs text-warning">
      {runtime.availability === "adapter_missing"
        ? `${runtime.label} CLI is installed but the ACP adapter is missing.`
        : runtime.availability === "adapter_outdated"
          ? `${runtime.label} ACP adapter is outdated — reinstall to continue.`
          : runtime.requiresExternalCli
            ? `${runtime.label} CLI is missing. ${runtime.installHint}`
            : `${runtime.label} is not installed.`}{" "}
      Visit Settings &gt; Agents to set it up.
    </p>
  );
}

/**
 * Model options for the definition dialog.
 *
 * "Automatic" only exists on shared compute (relay-mesh), where the mesh picks
 * the model; every other provider must name one, so the auto row is dropped.
 */
export function buildPersonaModelDropdownOptions({
  isRelayMesh,
  loading,
  loadingValue,
  options,
}: {
  isRelayMesh: boolean;
  loading: boolean;
  loadingValue: string;
  options: readonly { id: string; label: string }[];
}): PersonaDropdownOption[] {
  return buildModelDropdownOptions({
    allowCustom: !isRelayMesh,
    globalModel: undefined,
    loading,
    loadingValue,
    options,
  })
    .filter(
      (option) => isRelayMesh || option.value !== AUTO_MODEL_DROPDOWN_VALUE,
    )
    .map((option) =>
      isRelayMesh && option.value === AUTO_MODEL_DROPDOWN_VALUE
        ? { ...option, label: "Automatic" }
        : option,
    );
}
