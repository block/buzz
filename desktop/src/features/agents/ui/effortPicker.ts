import type {
  AcpConfigOptionValue,
  ManagedAgentBackend,
  RuntimeConfigSurface,
} from "@/shared/api/types";
import type { PersonaDropdownOption } from "./agentConfigOptions";
import { resolveModelCapabilities } from "./modelCapabilities";

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
 * and there's nothing to pick until `effortChoices` knows the model's levels.
 *
 * `visible` is the single gate the dialog renders on: local backend AND known
 * choices.
 */
export function effortPickerState({
  backend,
  effortOptions,
  currentEffort,
  storedEffort = null,
}: {
  backend: ManagedAgentBackend;
  effortOptions: EffortOptions;
  currentEffort: string | null;
  /** The saved effort; kept selectable even when the model doesn't list it. */
  storedEffort?: string | null;
}): {
  visible: boolean;
  options: PersonaDropdownOption[];
  selectValue: string;
  /** The stored level is selected but the model doesn't list it. */
  unlisted: boolean;
  /** The model isn't known yet, so support for `selectValue` isn't either. */
  unknownModel: boolean;
} {
  const listed = Array.isArray(effortOptions) ? effortOptions : [];
  const isListed = (value: string) =>
    listed.some((option) => option.value === value);
  // A saved level the model doesn't list stays a visible, clearable option
  // rather than reading as "Adapter default"; only the user may clear it.
  const stored = storedEffort?.trim() ?? "";
  const storedUnlisted = stored.length > 0 && !isListed(stored);
  const visible =
    backend.type === "local" &&
    (Array.isArray(effortOptions) || storedUnlisted);

  const options: PersonaDropdownOption[] = [
    { label: "Adapter default", value: EFFORT_DEFAULT_DROPDOWN_VALUE },
    ...listed.map((option) => ({
      label: option.displayName ?? option.value,
      value: option.value,
    })),
    ...(storedUnlisted ? [{ label: stored, value: stored }] : []),
  ];

  const trimmed = currentEffort?.trim() ?? "";
  const unlisted = storedUnlisted && trimmed === stored;
  const selectValue =
    unlisted || (trimmed.length > 0 && isListed(trimmed))
      ? trimmed
      : EFFORT_DEFAULT_DROPDOWN_VALUE;

  return {
    visible,
    options,
    selectValue,
    unlisted: unlisted && effortOptions !== EFFORT_LEVELS_UNKNOWN,
    unknownModel: unlisted && effortOptions === EFFORT_LEVELS_UNKNOWN,
  };
}

/**
 * Map a dropdown selection back to the persisted value sent as
 * `effortLevel` in the locked update payload: the sentinel clears effort
 * (null → adapter default), any other value is the explicit effort level.
 */
export function effortSelectionToPersistedValue(value: string): string | null {
  return value === EFFORT_DEFAULT_DROPDOWN_VALUE ? null : value;
}

/**
 * Claude will run some model, but none is known yet (discovery pending or
 * failed). Distinct from `undefined`, which means the model has no levels.
 */
export const EFFORT_LEVELS_UNKNOWN = "unknown";

/**
 * Offered effort levels; `undefined` = the model offers none,
 * `EFFORT_LEVELS_UNKNOWN` = the model isn't known yet. Both hide the picker
 * unless a level is stored, which stays visible and clearable.
 */
export type EffortOptions =
  | readonly AcpConfigOptionValue[]
  | typeof EFFORT_LEVELS_UNKNOWN
  | undefined;

/** Model ids in precedence order: explicit/persona, global, adapter default. */
export type EffortModels = readonly (string | null | undefined)[];

const CLAUDE_ALIAS_EFFORTS = ["low", "medium", "high"];

/**
 * Effort levels to offer for the model that will actually run, or `undefined`
 * to hide the picker (`EFFORT_LEVELS_UNKNOWN` while Claude's model is not yet
 * known). Claude levels always come from the capability manifest.
 * Other runtimes keep native-only behavior: the running session's own list
 * while the runtime is unchanged (`sessionApplies`). The first
 * non-blank of `models` wins. A blank id must never reach the manifest: its blank
 * fallback is adaptive and would invent levels for an unknown default.
 */
export function effortChoices({
  runtimeId,
  models,
  sessionApplies,
  session,
}: {
  runtimeId: string | undefined;
  models: EffortModels;
  sessionApplies: boolean;
  session?: RuntimeConfigSurface;
}): EffortOptions {
  // Claude's stored surface does not record which model its session ran, so
  // its levels cannot be trusted for the saved model; the manifest decides.
  if (
    runtimeId !== "claude" &&
    sessionApplies &&
    session?.effortConfigId !== undefined
  ) {
    return session.effortOptions ?? [];
  }
  if (runtimeId !== "claude") {
    return undefined;
  }
  const id = models.map((model) => model?.trim()).find(Boolean);
  if (!id) {
    return EFFORT_LEVELS_UNKNOWN;
  }
  const alias = id.toLowerCase().replace(/\[1m\]$/, "");
  const { thinkingMode, supportedEfforts } = resolveModelCapabilities(
    "anthropic",
    id,
  );
  const levels =
    alias === "opus" || alias === "sonnet"
      ? CLAUDE_ALIAS_EFFORTS
      : thinkingMode === "adaptive" || thinkingMode === "manual-budget"
        ? supportedEfforts
        : [];
  return levels.length > 0 ? levels.map((value) => ({ value })) : undefined;
}

/**
 * Whether a pending effort selection may be saved for the given choices. An
 * unknown model keeps the pick: the harness tolerates a level it rejects.
 */
export function isSavableEffort(
  level: string | null,
  choices: EffortOptions,
): boolean {
  return (
    level === null ||
    choices === EFFORT_LEVELS_UNKNOWN ||
    (choices ?? []).some((choice) => choice.value === level)
  );
}
