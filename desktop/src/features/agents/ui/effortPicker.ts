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

function humanizeEffortValue(value: string): string {
  if (!value) return value;
  return value.charAt(0).toUpperCase() + value.slice(1);
}

/**
 * Pure gating + option compute for the effort write control in the edit dialog.
 *
 * The picker is a LOCAL-only, Save-gated write control: the dialog embeds the
 * selection in the locked `update_managed_agent` payload (PR #4625), which the
 * Rust backend rejects for non-local backends (remote effort is set at deploy
 * time via `policy_env`). So the UI must not offer it for a provider backend.
 *
 * Visibility must NOT depend solely on a live session advertising
 * `thought_level` (`effortConfigId`). That discovery is flaky for the same
 * model (absent pre-first-session, while config surface loads, after idle
 * teardown). When the session has not advertised yet, fall back to static
 * capability / runtime-catalog values so effort stays available whenever the
 * model or harness supports it.
 *
 * `visible` = local backend AND (session configId OR fallback values present).
 */
export function effortPickerState({
  backend,
  effortConfigId,
  effortOptions,
  currentEffort,
  fallbackEffortValues,
}: {
  backend: ManagedAgentBackend;
  effortConfigId: string | undefined;
  effortOptions: readonly AcpConfigOptionValue[] | undefined;
  currentEffort: string | null;
  /**
   * Static effort values when the running session has not advertised options
   * yet (model-capabilities projection and/or runtime `effortCanonicalValues`).
   */
  fallbackEffortValues?: readonly string[];
}): {
  visible: boolean;
  options: PersonaDropdownOption[];
  selectValue: string;
} {
  const sessionOptions = effortOptions ?? [];
  const fallback = (fallbackEffortValues ?? []).filter(
    (value) => value.trim().length > 0,
  );
  const resolvedOptions: AcpConfigOptionValue[] =
    sessionOptions.length > 0
      ? [...sessionOptions]
      : fallback.map((value) => ({
          value,
          displayName: humanizeEffortValue(value),
        }));

  const visible =
    backend.type === "local" &&
    (effortConfigId !== undefined || resolvedOptions.length > 0);

  const options: PersonaDropdownOption[] = [
    { label: "Adapter default", value: EFFORT_DEFAULT_DROPDOWN_VALUE },
    ...resolvedOptions.map((option) => ({
      label: option.displayName ?? option.value,
      value: option.value,
    })),
  ];

  // Preselect the currently-configured effort when it maps to a known option;
  // otherwise fall back to the adapter-default sentinel (also the null case).
  const trimmed = currentEffort?.trim() ?? "";
  const selectValue =
    trimmed.length > 0 &&
    resolvedOptions.some((option) => option.value === trimmed)
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
