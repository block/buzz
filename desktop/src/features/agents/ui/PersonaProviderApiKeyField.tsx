import * as React from "react";
import { Eye, EyeOff } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { RequiredFieldLabel } from "./agentConfigControls";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./agentConfigOptions";

/**
 * Top-level API key pseudo-field for providers that require a secret
 * credential (anthropic → ANTHROPIC_API_KEY, openai → OPENAI_COMPAT_API_KEY).
 *
 * This is a pure view over `envVars[secretEnvVar]` — writes go through
 * `onValueChange`, which the parent routes to `setEnvVars`. No second copy
 * of the secret exists; the env-var row in Advanced and this field are the
 * same state, so sync is free.
 *
 * When the key is satisfied by an inherited layer (global, file, baked, or
 * persona snapshot), the field shows a placeholder instead of an empty
 * required field — consistent with `computeLocalModeGate`'s satisfied-key
 * logic. The inherited value is never echoed into the field.
 */
export function PersonaProviderApiKeyField({
  disabled,
  isInherited,
  inheritedLabel,
  isRequired,
  label,
  onValueChange,
  value,
}: {
  disabled: boolean;
  /** True when the key is satisfied by an inherited layer. */
  isInherited: boolean;
  /** Human-readable source of the inherited value. */
  inheritedLabel: string;
  /** True when the key is required and not satisfied anywhere. */
  isRequired: boolean;
  /** Display label, e.g. "API Key". */
  label: string;
  onValueChange: (next: string) => void;
  /** Current agent-local value of the secret env var. */
  value: string;
}) {
  const [showValue, setShowValue] = React.useState(false);
  const inputId = "persona-provider-api-key";

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
          data-testid="persona-provider-api-key"
          disabled={disabled}
          id={inputId}
          onChange={(event) => onValueChange(event.target.value)}
          placeholder={isInherited ? inheritedLabel : "Paste API key…"}
          type={showValue ? "text" : "password"}
          value={value}
        />
        <button
          aria-label={showValue ? "Hide API key" : "Show API key"}
          className="shrink-0 text-muted-foreground hover:text-foreground"
          onClick={() => setShowValue((v) => !v)}
          type="button"
        >
          {showValue ? (
            <EyeOff className="h-4 w-4" />
          ) : (
            <Eye className="h-4 w-4" />
          )}
        </button>
      </div>
    </div>
  );
}
