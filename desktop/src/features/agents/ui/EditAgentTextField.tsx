import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./agentConfigOptions";

/**
 * Single-line text input in the persona-field shell, shared by the
 * instance-edit dialog's plain text fields (agent name, command, custom
 * provider/model ids) so the shell markup lives in one place.
 */
export function EditAgentTextField({
  ariaLabel,
  containerClassName,
  disabled,
  id,
  label,
  onValueChange,
  placeholder,
  value,
}: {
  /** Accessible name for label-less inline inputs. */
  ariaLabel?: string;
  containerClassName?: string;
  disabled: boolean;
  id: string;
  /** Rendered above the shell; omit for inline inputs with an `ariaLabel`. */
  label?: string;
  onValueChange: (value: string) => void;
  placeholder: string;
  value: string;
}) {
  const shell = (
    <div
      className={cn(
        "flex min-h-11 items-center px-3",
        PERSONA_FIELD_SHELL_CLASS,
        label ? undefined : containerClassName,
      )}
    >
      <Input
        aria-label={ariaLabel}
        autoCorrect="off"
        className={cn("h-8 px-0 py-0 leading-6", PERSONA_FIELD_CONTROL_CLASS)}
        disabled={disabled}
        id={id}
        onChange={(event) => onValueChange(event.target.value)}
        placeholder={placeholder}
        value={value}
      />
    </div>
  );
  if (!label) {
    return shell;
  }
  return (
    <div className={cn("space-y-1.5", containerClassName)}>
      <label className="text-sm font-medium text-foreground" htmlFor={id}>
        {label}
      </label>
      {shell}
    </div>
  );
}
