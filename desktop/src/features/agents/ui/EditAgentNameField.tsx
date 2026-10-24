import { uiT } from "@/shared/i18n/uiTranslation";
import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import {
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_FIELD_CONTROL_CLASS,
} from "./agentConfigOptions";
export function EditAgentNameField({
  name,
  onChange,
  disabled,
}: {
  name: string;
  onChange: (value: string) => void;
  disabled: boolean;
}) {
  return (
    <div className="space-y-1.5">
      <label
        className="text-sm font-medium text-foreground"
        htmlFor="edit-agent-name"
      >
        {uiT("Agent name")}
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
          disabled={disabled}
          id="edit-agent-name"
          onChange={(event) => onChange(event.target.value)}
          placeholder={uiT("Agent name")}
          value={name}
        />
      </div>
    </div>
  );
}
