import { Input } from "@/shared/ui/input";
import { cn } from "@/shared/lib/cn";
import {
  type ProviderApiKeyConfig,
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./personaDialogPickers";

export function PersonaProviderApiKeyField({
  config,
  disabled,
  onChange,
  value,
}: {
  config: ProviderApiKeyConfig;
  disabled: boolean;
  onChange: (value: string) => void;
  value: string;
}) {
  return (
    <div className="mt-2 space-y-1.5">
      <label
        className="text-sm font-medium text-foreground"
        htmlFor="persona-provider-api-key"
      >
        {config.label}
      </label>
      <div
        className={cn(
          "flex min-h-11 items-center px-3",
          PERSONA_FIELD_SHELL_CLASS,
        )}
      >
        <Input
          autoCorrect="off"
          className={cn("h-8 px-0 py-0 leading-6", PERSONA_FIELD_CONTROL_CLASS)}
          data-testid="persona-provider-api-key"
          disabled={disabled}
          id="persona-provider-api-key"
          onChange={(event) => onChange(event.target.value)}
          placeholder={config.placeholder}
          value={value}
        />
      </div>
    </div>
  );
}
