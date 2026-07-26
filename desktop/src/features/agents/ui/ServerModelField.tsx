import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./agentConfigOptions";

/**
 * Free-text model entry for a server agent whose spawner advertised no catalog.
 *
 * A dropdown would be a lie there — this device cannot know what the host can
 * run — so the field degrades to plain text plus an explanation.
 */
export function ServerModelField({
  disabled,
  id,
  onChange,
  value,
}: {
  disabled: boolean;
  id: string;
  onChange: (next: string) => void;
  value: string;
}) {
  return (
    <>
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
          id={id}
          onChange={(event) => onChange(event.target.value)}
          placeholder="Model ID"
          value={value}
        />
      </div>
      <p className="text-xs text-muted-foreground">
        Model list unavailable from this server
      </p>
    </>
  );
}
