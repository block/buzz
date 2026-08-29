import type {
  AcpRuntimeCatalogEntry,
  UpdateManagedAgentInput,
} from "@/shared/api/types";

import {
  formatRuntimeOptionLabel,
  isRuntimeSelectableForConfiguration,
  runtimeSupportsLlmProviderSelection,
  type PersonaDropdownOption,
} from "./agentConfigOptions";
import { ADD_CUSTOM_HARNESS_OPTION } from "./addCustomHarness";
import { resolveAgentCommandUpdate } from "./personaRuntimeModel";

type RuntimeIdentityEntry = Pick<AcpRuntimeCatalogEntry, "command" | "id">;

/**
 * Prefer the Rust-projected runtime ID over legacy command inference. The
 * command path remains only for persisted records that predate runtime IDs.
 */
export function resolveSelectedRuntimeId(
  runtimes: readonly RuntimeIdentityEntry[],
  runtimeId: string | null | undefined,
  agentCommand: string,
) {
  const effectiveRuntimeId = runtimeId?.trim();
  const matched =
    runtimes.find((runtime) => runtime.id === effectiveRuntimeId) ??
    runtimes.find(
      (runtime) => runtime.command?.trim() === agentCommand.trim(),
    ) ??
    runtimes.find((runtime) => runtime.id === agentCommand.trim());
  return effectiveRuntimeId || matched?.id || "custom";
}

export function buildRuntimeDropdownOptions(
  runtimes: readonly AcpRuntimeCatalogEntry[],
  selectedRuntimeId: string,
): PersonaDropdownOption[] {
  const options: PersonaDropdownOption[] = [
    ...runtimes.map((runtime) => ({
      disabled:
        runtime.id !== selectedRuntimeId &&
        !isRuntimeSelectableForConfiguration(runtime),
      label: formatRuntimeOptionLabel(runtime),
      value: runtime.id,
    })),
    { label: "Custom command", value: "custom" },
  ];
  if (
    selectedRuntimeId &&
    selectedRuntimeId !== "custom" &&
    !options.some((option) => option.value === selectedRuntimeId)
  ) {
    options.push({
      label: `${selectedRuntimeId} (current)`,
      value: selectedRuntimeId,
    });
  }
  options.push(ADD_CUSTOM_HARNESS_OPTION);
  return options;
}

/** Existing unready pins survive unrelated edits; new catalog pins do not. */
export function canPersistRuntimePin({
  inheritHarness,
  runtimeTouched,
  selectedRuntime,
  selectedRuntimeId,
}: {
  inheritHarness: boolean;
  runtimeTouched: boolean;
  selectedRuntime: AcpRuntimeCatalogEntry | undefined;
  selectedRuntimeId: string;
}) {
  return (
    inheritHarness ||
    !runtimeTouched ||
    selectedRuntimeId === "custom" ||
    isRuntimeSelectableForConfiguration(selectedRuntime)
  );
}

/**
 * Resolves the runtime that will be active after saving. Inherit follows the
 * linked persona, then the already-derived global default, before the legacy
 * command fallback.
 */
export function resolveProspectiveRuntimeId({
  agentCommand,
  defaultRuntimeId,
  inheritHarness,
  linkedPersonaRuntime,
  runtimes,
  selectedRuntimeId,
}: {
  agentCommand: string;
  defaultRuntimeId: string | undefined;
  inheritHarness: boolean;
  linkedPersonaRuntime: string | null | undefined;
  runtimes: readonly RuntimeIdentityEntry[];
  selectedRuntimeId: string;
}) {
  if (!inheritHarness) {
    return (
      runtimes.find((runtime) => runtime.id === selectedRuntimeId)?.id ??
      selectedRuntimeId
    );
  }
  const personaRuntimeId = linkedPersonaRuntime?.trim();
  if (personaRuntimeId) {
    return (
      runtimes.find((runtime) => runtime.id === personaRuntimeId)?.id ??
      personaRuntimeId
    );
  }
  return (
    defaultRuntimeId ??
    runtimes.find((runtime) => runtime.command?.trim() === agentCommand.trim())
      ?.id ??
    runtimes.find((runtime) => runtime.id === agentCommand.trim())?.id ??
    ""
  );
}

/**
 * Builds the identity-only patch. Omitting every field preserves identity on
 * unrelated edits; null runtimeId is reserved for an explicit raw command.
 */
export function resolveRuntimeIdentityUpdate({
  agentCommand,
  agentCommandOverride,
  argsTouched,
  inheritHarness,
  originalAgentCommand,
  originalArgs,
  parsedArgs,
  prospectiveRuntimeId,
  runtimeTouched,
  selectedRuntimeId,
}: {
  agentCommand: string;
  agentCommandOverride: string | null;
  argsTouched: boolean;
  inheritHarness: boolean;
  originalAgentCommand: string;
  originalArgs: readonly string[];
  parsedArgs: string[];
  prospectiveRuntimeId: string;
  runtimeTouched: boolean;
  selectedRuntimeId: string;
}): Pick<
  UpdateManagedAgentInput,
  "agentArgs" | "agentCommand" | "harnessOverride" | "runtimeId"
> {
  const agentCommandChanged =
    agentCommand.trim() !== originalAgentCommand.trim();
  const agentCommandUpdate =
    runtimeTouched || agentCommandChanged
      ? resolveAgentCommandUpdate({
          inheritHarness,
          agentCommand,
          originalAgentCommand,
          agentCommandOverride,
        })
      : undefined;
  const runtimeIdentityChanged =
    runtimeTouched ||
    (!inheritHarness &&
      selectedRuntimeId === "custom" &&
      agentCommandUpdate != null);
  const runtimeId = runtimeIdentityChanged
    ? inheritHarness
      ? prospectiveRuntimeId || undefined
      : selectedRuntimeId === "custom"
        ? null
        : selectedRuntimeId
    : undefined;
  const agentArgs = runtimeIdentityChanged
    ? (!inheritHarness && selectedRuntimeId === "custom") || argsTouched
      ? parsedArgs
      : []
    : argsTouched && parsedArgs.join(",") !== originalArgs.join(",")
      ? parsedArgs
      : undefined;

  return {
    agentArgs,
    agentCommand: agentCommandUpdate,
    harnessOverride: runtimeIdentityChanged ? !inheritHarness : undefined,
    runtimeId,
  };
}

/**
 * Whether the runtime the dialog originally opened with supports LLM provider
 * selection. Resolved from the catalog (providerEnvVar) rather than runtime-id
 * allowlists; edit-state runtime ids mutate during selection changes and
 * cannot identify the original state.
 */
export function resolveOriginalRuntimeSupportsProvider(
  runtimes: readonly AcpRuntimeCatalogEntry[],
  agentRuntime: string | null | undefined,
  originalAgentCommand: string,
): boolean {
  const originalRuntimeId = resolveSelectedRuntimeId(
    runtimes,
    agentRuntime,
    originalAgentCommand,
  );
  return runtimeSupportsLlmProviderSelection(
    runtimes.find((entry) => entry.id === originalRuntimeId)?.providerEnvVar,
  );
}
