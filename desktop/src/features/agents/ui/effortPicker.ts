import type {
  AcpConfigOptionValue,
  ManagedAgentBackend,
  NormalizedField,
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
 * Pure gating + option compute for the effort write control.
 *
 * The picker is a LOCAL-only, Save-gated write control: the dialog embeds the
 * selection in the locked `update_managed_agent` payload (PR #4625), which the
 * Rust backend rejects for non-local backends (remote effort is set at deploy
 * time via `policy_env`). So the UI must not offer it for a provider backend,
 * and there's nothing to pick until `effortChoices` knows the model's levels.
 *
 * `visible` is the single gate the dialog renders on: local backend AND either
 * known choices or a stored level, which stays shown so it can be cleared.
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
  /**
   * Why the selected level may not apply: the model doesn't list it
   * (`unlisted`), or the model isn't known yet (`unknownModel`).
   */
  note: "unlisted" | "unknownModel" | null;
} {
  const listed = Array.isArray(effortOptions) ? effortOptions : [];
  const unknownModel = effortOptions === EFFORT_LEVELS_UNKNOWN;
  const isListed = (value: string) =>
    listed.some((option) => option.value === value);
  const stored = storedEffort?.trim() ?? "";
  const current = currentEffort?.trim() ?? "";
  // Unlisted levels that stay selectable rather than reading as "Adapter
  // default": a saved level, which only the user may clear, and while the
  // model is unknown, the pick Save keeps.
  const unlisted = [...new Set([stored, unknownModel ? current : ""])].filter(
    (value) => value.length > 0 && !isListed(value),
  );
  const visible =
    backend.type === "local" &&
    (Array.isArray(effortOptions) || stored.length > 0);

  const options: PersonaDropdownOption[] = [
    { label: "Adapter default", value: EFFORT_DEFAULT_DROPDOWN_VALUE },
    ...listed.map((option) => ({
      label: option.displayName ?? option.value,
      value: option.value,
    })),
    ...unlisted.map((value) => ({ label: value, value })),
  ];

  const selectedUnlisted = unlisted.includes(current);
  return {
    visible,
    options,
    selectValue:
      selectedUnlisted || (current.length > 0 && isListed(current))
        ? current
        : EFFORT_DEFAULT_DROPDOWN_VALUE,
    note: !selectedUnlisted ? null : unknownModel ? "unknownModel" : "unlisted",
  };
}

/**
 * The agent's own saved effort level from its config surface. A session,
 * definition, global, or config-file value is not the agent's to show as
 * stored or to clear.
 */
export function ownEffortLevel(
  field: NormalizedField | null | undefined,
): string | null {
  return field?.origin === "buzzExplicit" ? field.value : null;
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

/** Claude Code's model aliases, which the manifest does not resolve. */
const CLAUDE_ALIASES = new Set(["default", "opus", "opusplan", "sonnet"]);
const CLAUDE_ALIAS_EFFORTS = ["low", "medium", "high"];

/**
 * Effort levels to offer for the model that will actually run, or `undefined`
 * to hide the picker (`EFFORT_LEVELS_UNKNOWN` while Claude's model is not yet
 * known). Claude levels always come from the capability manifest, except for
 * Claude Code's aliases. A model the manifest doesn't know offers none.
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
  const levels = CLAUDE_ALIASES.has(alias)
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
