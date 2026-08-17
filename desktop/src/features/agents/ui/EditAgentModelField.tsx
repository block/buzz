import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { PersonaDropdownField } from "./PersonaDropdownField";
import {
  type PersonaDropdownOption,
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
} from "./agentConfigOptions";

/**
 * Model field for the instance edit form — the counterpart to
 * `PersonaModelField` on the definition form, split out of
 * `AgentInstanceEditDialog` for the same reason that one is separate.
 *
 * `disabled` covers the custom-model escape hatch as well as the dropdown, so
 * a caller that owns the value elsewhere (a linked definition) closes both
 * ways in. `discoveryLoading` disables only the dropdown, whose options are
 * what's still loading.
 */
export function EditAgentModelField({
  disabled,
  discoveryLoading,
  isRequired,
  model,
  onCustomModelChange,
  onModelValueChange,
  options,
  selectValue,
  showCustomModelInput,
  statusMessage,
}: {
  disabled: boolean;
  discoveryLoading: boolean;
  isRequired: boolean;
  model: string;
  onCustomModelChange: (value: string) => void;
  onModelValueChange: (value: string) => void;
  options: readonly PersonaDropdownOption[];
  selectValue: string;
  showCustomModelInput: boolean;
  statusMessage: string | null;
}) {
  return (
    <div className="space-y-1.5">
      <label
        className="text-sm font-medium text-foreground"
        htmlFor="edit-agent-model"
      >
        Model
        {isRequired ? (
          <span className="ml-1 text-destructive" aria-hidden="true">
            *
          </span>
        ) : (
          <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
        )}
      </label>
      <PersonaDropdownField
        disabled={disabled || discoveryLoading}
        id="edit-agent-model"
        onValueChange={onModelValueChange}
        options={options}
        placeholder="Default model"
        value={selectValue}
      />
      {showCustomModelInput ? (
        <div
          className={cn(
            "mt-2 flex min-h-11 items-center px-3",
            PERSONA_FIELD_SHELL_CLASS,
          )}
        >
          <Input
            aria-label="Custom model ID"
            autoCorrect="off"
            className={cn(
              "h-8 px-0 py-0 leading-6",
              PERSONA_FIELD_CONTROL_CLASS,
            )}
            disabled={disabled}
            id="edit-agent-custom-model"
            onChange={(event) => onCustomModelChange(event.target.value)}
            placeholder="Custom model ID"
            value={model}
          />
        </div>
      ) : null}
      {statusMessage ? (
        <p className="text-xs text-muted-foreground">{statusMessage}</p>
      ) : null}
    </div>
  );
}
