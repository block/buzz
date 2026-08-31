import type { AcpCommandCandidate } from "@/shared/api/acpCommands";
import { acpCommandPickerState } from "./acpCommandPicker";
import { PersonaDropdownField } from "./PersonaDropdownField";

type AcpCommandFieldProps = {
  candidates: AcpCommandCandidate[];
  disabled: boolean;
  onValueChange: (value: string) => void;
  value: string;
};

export function AcpCommandField({
  candidates,
  disabled,
  onValueChange,
  value,
}: AcpCommandFieldProps) {
  const picker = acpCommandPickerState(value, candidates);
  return (
    <div className="space-y-1.5">
      <label
        className="text-sm font-medium text-foreground"
        htmlFor="persona-acp-command"
      >
        ACP command
      </label>
      <PersonaDropdownField
        disabled={disabled}
        id="persona-acp-command"
        onValueChange={onValueChange}
        options={picker.options}
        placeholder="Choose an ACP command"
        value={picker.selectValue}
      />
      <p className="text-xs text-muted-foreground">
        Selects the ACP transport used when this agent is deployed.
      </p>
    </div>
  );
}
