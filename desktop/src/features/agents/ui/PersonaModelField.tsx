import type * as React from "react";
import { motion } from "motion/react";

import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { PersonaModelCombobox } from "./PersonaModelCombobox";
import type { PersonaModelDiscoveryStatus } from "./personaModelDiscoveryStatus";
import {
  type PersonaDropdownOption,
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
} from "./personaDialogPickers";

type PersonaModelFieldProps = {
  disabled: boolean;
  isExplicitModelRequired: boolean;
  model: string;
  modelDiscoveryStatus: PersonaModelDiscoveryStatus | null;
  modelDropdownOptions: readonly PersonaDropdownOption[];
  modelSelectValue: string;
  onCustomModelChange: (value: string) => void;
  onModelValueChange: (value: string) => void;
  showCustomModelInput: boolean;
  transition: React.ComponentProps<typeof motion.div>["transition"];
};

export function PersonaModelField({
  disabled,
  isExplicitModelRequired,
  model,
  modelDiscoveryStatus,
  modelDropdownOptions,
  modelSelectValue,
  onCustomModelChange,
  onModelValueChange,
  showCustomModelInput,
  transition,
}: PersonaModelFieldProps) {
  return (
    <motion.div
      animate={{ height: "auto", opacity: 1, scale: 1 }}
      className="origin-top overflow-hidden"
      exit={{ height: 0, opacity: 0, scale: 0.98 }}
      initial={{ height: 0, opacity: 0, scale: 0.98 }}
      key="persona-model-field"
      transition={transition}
    >
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="persona-model"
        >
          Model
          {!isExplicitModelRequired ? (
            <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
          ) : null}
        </label>
        <PersonaModelCombobox
          disabled={disabled}
          id="persona-model"
          onValueChange={onModelValueChange}
          options={modelDropdownOptions}
          placeholder={
            isExplicitModelRequired ? "Choose a model" : "Default model"
          }
          value={modelSelectValue}
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
              id="persona-custom-model"
              onChange={(event) => onCustomModelChange(event.target.value)}
              placeholder="Custom model ID"
              value={model}
            />
          </div>
        ) : null}
        {modelDiscoveryStatus ? (
          <p
            aria-live="polite"
            className={cn(
              "text-xs",
              modelDiscoveryStatus.tone === "warning"
                ? "text-warning"
                : "text-muted-foreground",
            )}
            data-testid="persona-model-discovery-status"
          >
            {modelDiscoveryStatus.message}
          </p>
        ) : null}
      </div>
    </motion.div>
  );
}
