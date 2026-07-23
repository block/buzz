import { cn } from "@/shared/lib/cn";
import { Textarea } from "@/shared/ui/textarea";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
} from "./agentConfigOptions";

export function AgentPersonalityField({
  disabled,
  id,
  onChange,
  value,
}: {
  disabled: boolean;
  id: string;
  onChange: (value: string) => void;
  value: string;
}) {
  return (
    <div className="space-y-1.5">
      <label className="text-sm font-medium text-foreground" htmlFor={id}>
        Personality
        <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
      </label>
      <div className={PERSONA_FIELD_SHELL_CLASS}>
        <Textarea
          className={cn(
            "min-h-32 resize-y px-3 py-3 leading-5",
            PERSONA_FIELD_CONTROL_CLASS,
          )}
          disabled={disabled}
          id={id}
          onChange={(event) => onChange(event.target.value)}
          placeholder="Warm, curious, candid, and concise. Ask before making assumptions."
          value={value}
        />
      </div>
      <p className="text-xs text-muted-foreground">
        Describe how this agent thinks, speaks, and behaves. This guides every
        conversation.
      </p>
    </div>
  );
}
