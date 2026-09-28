import type {
  AcpRuntimeCatalogEntry,
  ManagedAgent,
  RuntimeConfigSurface,
} from "@/shared/api/types";
import { PERSONA_LABEL_OPTIONAL_CLASS } from "./agentConfigOptions";
import { getProviderEffortConfig } from "./buzzAgentConfig";
import {
  effortPickerState,
  effortSelectionToPersistedValue,
} from "./effortPicker";
import { PersonaDropdownField } from "./PersonaDropdownField";

/**
 * Thinking-effort write control for the edit dialog.
 *
 * Local-only by construction: the Rust backend rejects effort writes for
 * non-local backends (remote effort is set at deploy time via `policy_env`).
 *
 * Options prefer the running session's advertised `thought_level` values when
 * present; otherwise fall back to runtime `effortCanonicalValues` and the
 * static model-capabilities projection so the control does not disappear for
 * the same model between sessions / while config surface is loading.
 *
 * Save-gated, not direct-write: the control is fully controlled by the parent
 * dialog (`value`/`onChange`) and owns no mutation. The dialog persists the
 * selection by embedding `effortLevel` in the locked `update_managed_agent`
 * call (PR #4625), so the effort write is atomic with any access-policy change
 * and can never race or survive a Cancel/failed Save.
 */
export function EffortPickerField({
  agent,
  config,
  disabled,
  value,
  onChange,
  provider,
  model,
  selectedRuntime,
}: {
  agent: ManagedAgent;
  config: RuntimeConfigSurface | undefined;
  disabled: boolean;
  /** The pending persisted effort form (`null` = adapter default). */
  value: string | null;
  onChange: (level: string | null) => void;
  /** Effective provider id for capability fallback when session options absent. */
  provider?: string;
  /** Effective model id for capability fallback when session options absent. */
  model?: string;
  /** Catalog runtime — supplies harness-native effort vocab when present. */
  selectedRuntime?: AcpRuntimeCatalogEntry | null;
}) {
  const capabilityFallback = getProviderEffortConfig(
    provider ?? agent.provider ?? "",
    model ?? agent.model ?? "",
  ).validValues;
  const runtimeFallback = selectedRuntime?.effortCanonicalValues ?? [];
  const fallbackEffortValues =
    runtimeFallback.length > 0 ? runtimeFallback : [...capabilityFallback];

  const { visible, options, selectValue } = effortPickerState({
    backend: agent.backend,
    effortConfigId: config?.effortConfigId,
    effortOptions: config?.effortOptions,
    currentEffort: value,
    fallbackEffortValues,
  });

  if (!visible) {
    return null;
  }

  return (
    <div className="space-y-1.5">
      <label
        className="text-sm font-medium text-foreground"
        htmlFor="edit-agent-effort"
      >
        Thinking effort
        <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
      </label>
      <PersonaDropdownField
        disabled={disabled}
        id="edit-agent-effort"
        onValueChange={(next) =>
          onChange(effortSelectionToPersistedValue(next))
        }
        options={options}
        placeholder="Adapter default"
        value={selectValue}
      />
      <p className="text-xs text-muted-foreground">
        Applied at the next session start.
      </p>
    </div>
  );
}
