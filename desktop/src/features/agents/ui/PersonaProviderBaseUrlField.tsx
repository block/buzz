import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { RequiredFieldLabel } from "./agentConfigControls";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./agentConfigOptions";

/** Validate that a non-empty string is parseable as a URL. */
export function isValidUrl(value: string): boolean {
  const trimmed = value.trim();
  if (trimmed.length === 0) return false;
  try {
    // eslint-disable-next-line no-new
    new URL(trimmed);
    return true;
  } catch {
    return false;
  }
}

/**
 * Base URL pseudo-field for the OpenAI-compatible provider.
 *
 * Mirrors PersonaProviderApiKeyField: it is a pure view over an env var
 * (OPENAI_COMPAT_BASE_URL) and shares the same styled shell. The value is
 * not a secret, so it renders as a plain URL input with format validation.
 */
export function PersonaProviderBaseUrlField({
  disabled,
  isRequired,
  label,
  onValueChange,
  value,
}: {
  disabled: boolean;
  /** True when the base URL must be provided for the selected provider. */
  isRequired: boolean;
  /** Display label, e.g. "OpenAI-compatible base URL". */
  label: string;
  onValueChange: (next: string) => void;
  /** Current agent-local value of the base URL env var. */
  value: string;
}) {
  const [touched, setTouched] = React.useState(false);
  const inputId = "persona-provider-base-url";
  const trimmedValue = value.trim();
  const showError =
    touched && trimmedValue.length > 0 && !isValidUrl(trimmedValue);

  return (
    <div className="space-y-1.5">
      <RequiredFieldLabel htmlFor={inputId} isRequired={isRequired}>
        {label}
      </RequiredFieldLabel>
      <div
        className={cn(
          "flex min-h-11 items-center gap-2 px-3",
          PERSONA_FIELD_SHELL_CLASS,
        )}
      >
        <Input
          autoComplete="off"
          className={cn(
            "h-8 flex-1 px-0 py-0 leading-6",
            PERSONA_FIELD_CONTROL_CLASS,
          )}
          data-testid="persona-provider-base-url"
          disabled={disabled}
          id={inputId}
          onBlur={() => setTouched(true)}
          onChange={(event) => onValueChange(event.target.value)}
          placeholder="Paste OpenAI-compatible base URL"
          type="url"
          value={value}
        />
      </div>
      {showError ? (
        <p
          className="text-sm text-destructive"
          data-testid="persona-provider-base-url-error"
        >
          Enter a valid URL (e.g. https://api.venice.ai/v1).
        </p>
      ) : null}
    </div>
  );
}
