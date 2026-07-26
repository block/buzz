import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
  type PersonaDropdownOption,
} from "./agentConfigOptions";
import { RequiredFieldLabel } from "./agentConfigControls";
import { PersonaDropdownField } from "./PersonaDropdownField";

/**
 * "LLM provider" dropdown plus its custom-id escape hatch.
 *
 * Shared by the instance and definition dialogs so the two stay in step — for a
 * server-hosted agent both are handed the spawner's advertised provider list
 * instead of the locally-derived one.
 */
export function LlmProviderField({
  customInputId,
  disabled,
  id,
  isRequired,
  onProviderTextChange,
  onValueChange,
  options,
  placeholder,
  providerValue,
  selectValue,
  showCustomInput,
}: {
  customInputId: string;
  disabled: boolean;
  id: string;
  isRequired: boolean;
  onProviderTextChange: (next: string) => void;
  onValueChange: (next: string) => void;
  options: PersonaDropdownOption[];
  placeholder: string;
  providerValue: string;
  selectValue: string;
  showCustomInput: boolean;
}) {
  return (
    <div className="space-y-1.5">
      <RequiredFieldLabel
        className="text-foreground"
        htmlFor={id}
        isRequired={isRequired}
      >
        LLM provider
        {isRequired ? null : (
          <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
        )}
      </RequiredFieldLabel>
      <PersonaDropdownField
        disabled={disabled}
        id={id}
        onValueChange={onValueChange}
        options={options}
        placeholder={placeholder}
        value={selectValue}
      />
      {showCustomInput ? (
        <div
          className={cn(
            "mt-2 flex min-h-11 items-center px-3",
            PERSONA_FIELD_SHELL_CLASS,
          )}
        >
          <Input
            aria-label="Custom provider ID"
            autoCorrect="off"
            className={cn(
              "h-8 px-0 py-0 leading-6",
              PERSONA_FIELD_CONTROL_CLASS,
            )}
            disabled={disabled}
            id={customInputId}
            onChange={(event) => onProviderTextChange(event.target.value)}
            placeholder="Custom provider ID"
            value={providerValue}
          />
        </div>
      ) : null}
    </div>
  );
}
