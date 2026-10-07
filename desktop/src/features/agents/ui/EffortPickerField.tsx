import type { ManagedAgentBackend } from "@/shared/api/types";
import { PERSONA_LABEL_OPTIONAL_CLASS } from "./agentConfigOptions";
import {
  type EffortOptions,
  effortPickerState,
  effortSelectionToPersistedValue,
} from "./effortPicker";
import { PersonaDropdownField } from "./PersonaDropdownField";

/**
 * Thinking-effort write control for the agent edit and create dialogs.
 *
 * Local-only by construction: the Rust backend rejects effort writes for
 * non-local backends (remote effort is set at deploy time via `policy_env`). So the
 * control renders only for a local backend AND when `effortChoices` knows the
 * levels for the model that will run (Claude model data for Claude, the running
 * session's list otherwise) or a level is stored. The read-only
 * configured-vs-running two-facts display lives in `AgentConfigPanel`; this is
 * the write control.
 *
 * Save-gated, not direct-write: the control is fully controlled by the parent
 * dialog (`value`/`onChange`) and owns no mutation. The dialog persists the
 * selection in its save payload (in Edit, the locked `update_managed_agent`
 * call, PR #4625), so the effort write is atomic with any access-policy change
 * and can never race or survive a Cancel/failed Save.
 */
export function EffortPickerField({
  backend,
  choices,
  disabled,
  value,
  storedEffort,
  onChange,
}: {
  backend: ManagedAgentBackend;
  /** Levels from `effortChoices`; `undefined` hides the control. */
  choices: EffortOptions;
  disabled: boolean;
  /** The pending persisted effort form (`null` = adapter default). */
  value: string | null;
  /** The saved effort, kept selectable when the model doesn't list it. */
  storedEffort?: string | null;
  onChange: (level: string | null) => void;
}) {
  const { visible, options, selectValue, note } = effortPickerState({
    backend,
    effortOptions: choices,
    currentEffort: value,
    storedEffort,
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
        {note === "unknownModel"
          ? `Support for ${selectValue} isn't known yet. `
          : note === "unlisted"
            ? `This model may not support ${selectValue}. `
            : null}
        Applied at the next session start.
      </p>
    </div>
  );
}
