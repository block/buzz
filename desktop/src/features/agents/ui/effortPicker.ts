import type {
  AcpConfigOptionValue,
  ManagedAgentBackend,
} from "@/shared/api/types";
import type { PersonaDropdownOption } from "./agentConfigOptions";

/**
 * Sentinel dropdown value for "no explicit effort" — reverts the agent to the
 * adapter default at the next spawn. Distinct from any adapter option value.
 */
export const EFFORT_DEFAULT_DROPDOWN_VALUE = "__effort_default__";

/**
 * Pure gating + option compute for the effort write control in the edit dialog.
 *
 * The picker is a LOCAL-only, Save-gated write control: the dialog embeds the
 * selection in the locked `update_managed_agent` payload (PR #4625), which the
 * Rust backend rejects for non-local backends (remote effort is set at deploy
 * time via `policy_env`). So the UI must not offer it for a provider backend,
 * and there's nothing to pick until the adapter has advertised a `thought_level`
 * config option (discovered from the running session — `effortConfigId` is
 * absent pre-first-session and for runtimes/models that don't support effort).
 *
 * `visible` is the single gate the dialog renders on: local backend AND a
 * discovered `effortConfigId`.
 */
export function effortPickerState({
  backend,
  effortConfigId,
  effortOptions,
  currentEffort,
}: {
  backend: ManagedAgentBackend;
  effortConfigId: string | undefined;
  effortOptions: readonly AcpConfigOptionValue[] | undefined;
  currentEffort: string | null;
}): {
  visible: boolean;
  options: PersonaDropdownOption[];
  selectValue: string;
} {
  const visible = backend.type === "local" && effortConfigId !== undefined;

  const options: PersonaDropdownOption[] = [
    { label: "Adapter default", value: EFFORT_DEFAULT_DROPDOWN_VALUE },
    ...(effortOptions ?? []).map((option) => ({
      label: option.displayName ?? option.value,
      value: option.value,
    })),
  ];

  // Preselect the currently-configured effort when it maps to a known option;
  // otherwise fall back to the adapter-default sentinel (also the null case).
  const trimmed = currentEffort?.trim() ?? "";
  const selectValue =
    trimmed.length > 0 &&
    (effortOptions ?? []).some((option) => option.value === trimmed)
      ? trimmed
      : EFFORT_DEFAULT_DROPDOWN_VALUE;

  return { visible, options, selectValue };
}

/**
 * Map a dropdown selection back to the persisted value sent as
 * `effortLevel` in the locked update payload: the sentinel clears effort
 * (null → adapter default), any other value is the explicit effort level.
 */
export function effortSelectionToPersistedValue(value: string): string | null {
  return value === EFFORT_DEFAULT_DROPDOWN_VALUE ? null : value;
}
