import { Input } from "@/shared/ui/input";
import { cn } from "@/shared/lib/cn";
import { EnvVarsEditor, type EnvVarsValue } from "./EnvVarsEditor";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./personaDialogPickers";

const PERSONA_LABEL_OPTIONAL_CLASS =
  "ml-1 text-xs font-normal text-muted-foreground/50";

export function PersonaAdvancedFields({
  disabled,
  envVars,
  namePoolText,
  onEnvVarsChange,
  onNamePoolTextChange,
  requiredEnvKeys = [],
  fileSatisfiedEnvKeys = [],
}: {
  disabled: boolean;
  envVars: EnvVarsValue;
  namePoolText: string;
  onEnvVarsChange: (value: EnvVarsValue) => void;
  onNamePoolTextChange: (value: string) => void;
  requiredEnvKeys?: readonly string[];
  fileSatisfiedEnvKeys?: readonly string[];
}) {
  return (
    <div className="space-y-5 pt-2">
      <div className="space-y-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="persona-name-pool"
        >
          Instance name pool
          <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
        </label>
        <div
          className={cn(
            "flex min-h-11 items-center px-3",
            PERSONA_FIELD_SHELL_CLASS,
          )}
        >
          <Input
            autoCapitalize="words"
            autoCorrect="off"
            className={cn(
              "h-8 px-0 py-0 leading-6",
              PERSONA_FIELD_CONTROL_CLASS,
            )}
            disabled={disabled}
            id="persona-name-pool"
            onChange={(event) => onNamePoolTextChange(event.target.value)}
            placeholder="Birch, Compass, Ridge, Thistle"
            spellCheck={false}
            value={namePoolText}
          />
        </div>
      </div>

      <EnvVarsEditor
        disabled={disabled}
        fileSatisfiedKeys={fileSatisfiedEnvKeys}
        onChange={onEnvVarsChange}
        requiredKeys={requiredEnvKeys}
        value={envVars}
      />
    </div>
  );
}
