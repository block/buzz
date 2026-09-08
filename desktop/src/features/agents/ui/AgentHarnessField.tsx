import type { ReactNode } from "react";

import type { PersonaDropdownOption } from "./agentConfigOptions";
import { PersonaDropdownField } from "./PersonaDropdownField";
import { HarnessCatalogRetryNotice } from "./HarnessCatalogRetryNotice";

export function AgentHarnessField({
  catalogStatus,
  disabled,
  onValueChange,
  options,
  placeholder,
  value,
  warning,
}: {
  catalogStatus?: "loading" | "ready" | "error";
  disabled: boolean;
  onValueChange: (value: string) => void;
  options: PersonaDropdownOption[];
  placeholder: string;
  value: string;
  warning?: ReactNode;
}) {
  return (
    <div className="space-y-1.5">
      <label
        className="text-sm font-medium text-foreground"
        htmlFor="persona-runtime"
      >
        Agent harness
      </label>
      <PersonaDropdownField
        disabled={disabled}
        id="persona-runtime"
        onValueChange={onValueChange}
        options={options}
        placeholder={placeholder}
        value={value}
      />
      {catalogStatus === "error" ? <HarnessCatalogRetryNotice /> : warning}
    </div>
  );
}
