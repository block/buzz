import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { RequiredFieldLabel } from "./agentConfigControls";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
  PERSONA_LABEL_OPTIONAL_CLASS,
} from "./agentConfigOptions";

/**
 * Top-level OpenAI-compatible base URL field.
 *
 * Pure view over `envVars[OPENAI_COMPAT_BASE_URL]` — writes go through
 * `onValueChange` into the same env_vars map used by Advanced. Blank is valid
 * when unset (native default / backward compat); a whitespace-only value typed
 * into the field is rejected so requests don't silently hit api.openai.com.
 * Invalid (non-URL) values surface an inline error.
 */
export function PersonaProviderBaseUrlField({
  disabled,
  errorMessage,
  isInherited,
  inheritedLabel,
  isInvalid,
  label = "OpenAI-compatible base URL",
  onValueChange,
  value,
}: {
  disabled: boolean;
  /** Override the default validation error text (e.g. blank-rejected message). */
  errorMessage?: string;
  /** True when the URL is satisfied by an inherited layer. */
  isInherited: boolean;
  /** Human-readable source of the inherited value. */
  inheritedLabel: string;
  /** True when the local value fails validation (bad URL or blank). */
  isInvalid: boolean;
  /** Display label. */
  label?: string;
  onValueChange: (next: string) => void;
  /** Current agent-local value of the base URL env var. */
  value: string;
}) {
  const inputId = "persona-provider-base-url";
  const errorId = `${inputId}-error`;

  return (
    <div className="space-y-1.5">
      <RequiredFieldLabel htmlFor={inputId} isRequired={false}>
        {label}
        <span className={PERSONA_LABEL_OPTIONAL_CLASS}>Optional</span>
      </RequiredFieldLabel>
      <div
        className={cn(
          "flex min-h-11 items-center gap-2 px-3",
          PERSONA_FIELD_SHELL_CLASS,
          isInvalid && "border-destructive/60 focus-within:border-destructive",
        )}
      >
        <Input
          aria-describedby={isInvalid ? errorId : undefined}
          aria-invalid={isInvalid || undefined}
          autoComplete="off"
          autoCorrect="off"
          className={cn(
            "h-8 flex-1 px-0 py-0 leading-6",
            PERSONA_FIELD_CONTROL_CLASS,
          )}
          data-testid="persona-provider-base-url"
          disabled={disabled}
          id={inputId}
          onChange={(event) => onValueChange(event.target.value)}
          placeholder={
            isInherited ? inheritedLabel : "https://api.openai.com/v1"
          }
          spellCheck={false}
          type="url"
          value={value}
        />
      </div>
      {isInvalid ? (
        <p
          className="text-xs text-destructive"
          data-testid="persona-provider-base-url-error"
          id={errorId}
          role="alert"
        >
          {errorMessage ?? "Enter a valid http:// or https:// base URL."}
        </p>
      ) : null}
    </div>
  );
}
