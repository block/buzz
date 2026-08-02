import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";

import {
  type PersonaDropdownOption,
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
} from "./agentConfigOptions";
import { RequiredFieldLabel } from "./agentConfigControls";
import { PersonaDropdownField } from "./PersonaDropdownField";

type AgentLlmProviderFieldProps = {
  disabled: boolean;
  isRequired: boolean;
  onProviderValueChange: (next: string) => void;
  onCustomProviderChange: (next: string) => void;
  options: PersonaDropdownOption[];
  provider: string;
  selectValue: string;
  showCustomInput: boolean;
};

/**
 * The "LLM provider" row: a dropdown of the harness's known providers plus the
 * free-text box a "Custom provider..." selection reveals.
 */
export function AgentLlmProviderField({
  disabled,
  isRequired,
  onCustomProviderChange,
  onProviderValueChange,
  options,
  provider,
  selectValue,
  showCustomInput,
}: AgentLlmProviderFieldProps) {
  return (
    <div className="space-y-1.5">
      <RequiredFieldLabel
        htmlFor="persona-llm-provider"
        isRequired={isRequired}
      >
        LLM provider
        {!isRequired ? (
          <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
        ) : null}
      </RequiredFieldLabel>
      <PersonaDropdownField
        disabled={disabled}
        id="persona-llm-provider"
        onValueChange={onProviderValueChange}
        options={options}
        placeholder="Choose a provider"
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
            id="persona-custom-provider"
            onChange={(event) => onCustomProviderChange(event.target.value)}
            placeholder="Custom provider ID"
            value={provider}
          />
        </div>
      ) : null}
    </div>
  );
}
