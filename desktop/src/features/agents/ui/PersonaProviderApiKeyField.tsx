import * as React from "react";
import { Eye, EyeOff } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { Button } from "@/shared/ui/button";
import { RequiredFieldLabel } from "./agentConfigControls";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./agentConfigOptions";

/**
 * Top-level API key pseudo-field for providers that require a secret
 * credential (anthropic → ANTHROPIC_API_KEY, openai → OPENAI_COMPAT_API_KEY).
 *
 * Ordinary credentials are a pure view over `envVars[secretEnvVar]`: writes
 * go through `onValueChange`, and the Advanced row shares that same state.
 * When `deviceSecret` is present, the input instead holds only a transient
 * draft. Save/remove call the keyring-backed provider-secret commands, while
 * the UI receives presence/source metadata and never the stored value.
 *
 * When the key is satisfied by an inherited layer (global, file, baked, or
 * persona snapshot), the field shows a placeholder instead of an empty
 * required field — consistent with `computeLocalModeGate`'s satisfied-key
 * logic. The inherited value is never echoed into the field.
 */
export function PersonaProviderApiKeyField({
  disabled,
  envVarName,
  isInherited,
  inheritedLabel,
  isRequired,
  label,
  onValueChange,
  value,
  deviceSecret,
}: {
  disabled: boolean;
  /**
   * The backing environment variable name, e.g. `OPENAI_COMPAT_API_KEY`.
   * Rendered as a monospace hint beneath the label so users can distinguish
   * this field from other keys with similar names (e.g. `OPENAI_API_KEY`).
   * When present, the input's `aria-describedby` points at the hint element.
   */
  envVarName?: string;
  /** True when the key is satisfied by an inherited layer. */
  isInherited: boolean;
  /** Human-readable source of the inherited value. */
  inheritedLabel: string;
  /** True when the key is required and not satisfied anywhere. */
  isRequired: boolean;
  /** Display label, e.g. "Anthropic API Key". */
  label: string;
  onValueChange: (next: string) => void;
  /** Current agent-local value of the secret env var. */
  value: string;
  deviceSecret?: {
    configured: boolean;
    isPending: boolean;
    source: "environment" | "keyring" | "missing" | "unavailable" | null;
    error: unknown;
    restartedCount: number;
    failedRestartCount: number;
    set: (value: string) => Promise<unknown>;
    clear: () => Promise<unknown>;
  };
}) {
  const [showValue, setShowValue] = React.useState(false);
  const [deviceDraft, setDeviceDraft] = React.useState("");
  const uid = React.useId();
  const inputId = `persona-provider-api-key-${uid}`;
  const hintId = envVarName
    ? `persona-provider-api-key-hint-${uid}`
    : undefined;

  return (
    <div className="space-y-1.5">
      <RequiredFieldLabel htmlFor={inputId} isRequired={isRequired}>
        {label}
      </RequiredFieldLabel>
      {envVarName ? (
        <p className="text-xs text-muted-foreground font-mono" id={hintId}>
          {envVarName}
        </p>
      ) : null}
      <div
        className={cn(
          "flex min-h-11 items-center gap-2 px-3",
          PERSONA_FIELD_SHELL_CLASS,
        )}
      >
        <Input
          aria-describedby={hintId}
          autoComplete="off"
          className={cn(
            "h-8 flex-1 px-0 py-0 leading-6",
            PERSONA_FIELD_CONTROL_CLASS,
          )}
          data-testid="persona-provider-api-key"
          disabled={disabled}
          id={inputId}
          onChange={(event) =>
            deviceSecret
              ? setDeviceDraft(event.target.value)
              : onValueChange(event.target.value)
          }
          placeholder={
            deviceSecret?.configured
              ? deviceSecret.source === "environment"
                ? "Provided by environment"
                : "Saved securely on this device"
              : isInherited
                ? inheritedLabel
                : "Paste API key…"
          }
          type={showValue ? "text" : "password"}
          value={deviceSecret ? deviceDraft : value}
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
      {deviceSecret ? (
        <div className="flex items-center gap-2">
          <Button
            disabled={deviceSecret.isPending || deviceDraft.trim().length === 0}
            onClick={async () => {
              await deviceSecret.set(deviceDraft);
              setDeviceDraft("");
            }}
            size="sm"
            type="button"
            variant="outline"
          >
            {deviceSecret.configured ? "Replace" : "Save securely"}
          </Button>
          {deviceSecret.configured && deviceSecret.source !== "environment" ? (
            <Button
              disabled={deviceSecret.isPending}
              onClick={() => deviceSecret.clear()}
              size="sm"
              type="button"
              variant="ghost"
            >
              Remove
            </Button>
          ) : null}
          {deviceSecret.source === "unavailable" ? (
            <span className="text-xs text-destructive">
              Secure storage is unavailable.
            </span>
          ) : null}
          {deviceSecret.error ? (
            <span className="text-xs text-destructive">
              Could not update this credential.
            </span>
          ) : null}
          {deviceSecret.restartedCount > 0 ? (
            <span className="text-xs text-muted-foreground">
              Restarted {deviceSecret.restartedCount} running agent
              {deviceSecret.restartedCount === 1 ? "" : "s"}.
            </span>
          ) : null}
          {deviceSecret.failedRestartCount > 0 ? (
            <span className="text-xs text-destructive">
              {deviceSecret.failedRestartCount} agent
              {deviceSecret.failedRestartCount === 1 ? "" : "s"} could not
              restart.
            </span>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
