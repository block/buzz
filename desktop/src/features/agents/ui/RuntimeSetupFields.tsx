import type { RuntimeSetupField } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { RequiredFieldLabel } from "./agentConfigControls";
import {
  isRuntimeSetupFieldValueValid,
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./agentConfigOptions";
import type { EnvVarsValue } from "./EnvVarsEditor";

export function runtimeSetupFieldState({
  configuredByKeys = [],
  field,
  inheritedFrom,
  inheritedConfiguredByKeys = [],
  value,
}: {
  configuredByKeys?: readonly string[];
  field: RuntimeSetupField;
  inheritedFrom?: EnvVarsValue;
  inheritedConfiguredByKeys?: readonly string[];
  value: EnvVarsValue;
}) {
  const hasLocalValue = field.envKey in value;
  const localValue = value[field.envKey] ?? "";
  const configured = !hasLocalValue && configuredByKeys.includes(field.envKey);
  const inherited =
    !hasLocalValue &&
    !configured &&
    (inheritedConfiguredByKeys.includes(field.envKey) ||
      isRuntimeSetupFieldValueValid(field, inheritedFrom?.[field.envKey]));
  const valid = hasLocalValue
    ? isRuntimeSetupFieldValueValid(field, localValue)
    : configured || inherited || !field.required;
  return { configured, hasLocalValue, inherited, localValue, valid };
}

function validationMessage(field: RuntimeSetupField): string {
  switch (field.validation) {
    case "tailnet_https_origin":
      return "Enter a Tailnet HTTPS origin ending in .ts.net, with no path, credentials, query, or fragment.";
    case "app_password":
      return "App password must be at least 32 characters.";
  }
}

/** Catalog-driven setup controls backed directly by the agent env map. */
export function RuntimeSetupFields({
  configuredByKeys,
  disabled = false,
  fields,
  inheritedFrom,
  inheritedConfiguredByKeys,
  inheritedLabel = "defaults",
  onChange,
  secureStorageUnavailable = false,
  value,
}: {
  configuredByKeys?: readonly string[];
  disabled?: boolean;
  fields: readonly RuntimeSetupField[];
  inheritedFrom?: EnvVarsValue;
  inheritedConfiguredByKeys?: readonly string[];
  inheritedLabel?: string;
  onChange: (next: EnvVarsValue) => void;
  secureStorageUnavailable?: boolean;
  value: EnvVarsValue;
}) {
  if (fields.length === 0) return null;

  return (
    <div className="space-y-4" data-testid="runtime-setup-fields">
      {fields.map((field) => {
        const state = runtimeSetupFieldState({
          configuredByKeys,
          field,
          inheritedFrom,
          inheritedConfiguredByKeys,
          value,
        });
        const inputId = `runtime-setup-${field.envKey.toLowerCase().replaceAll("_", "-")}`;
        const helperId = `${inputId}-helper`;
        return (
          <div className="space-y-1.5" key={field.envKey}>
            <RequiredFieldLabel
              htmlFor={inputId}
              isRequired={field.required && !state.valid}
            >
              {field.label}
            </RequiredFieldLabel>
            <p className="font-mono text-xs text-muted-foreground">
              {field.envKey}
            </p>
            <div
              className={cn(
                "flex min-h-11 items-center px-3",
                PERSONA_FIELD_SHELL_CLASS,
                !state.valid && "border-destructive/50",
              )}
            >
              <Input
                aria-describedby={helperId}
                autoCapitalize="none"
                autoComplete={field.kind === "secret" ? "new-password" : "url"}
                autoCorrect="off"
                className={cn(
                  "h-8 px-0 py-0 leading-6",
                  PERSONA_FIELD_CONTROL_CLASS,
                )}
                disabled={disabled}
                id={inputId}
                onChange={(event) => {
                  const next = { ...value };
                  if (event.target.value.length === 0) {
                    delete next[field.envKey];
                  } else {
                    next[field.envKey] = event.target.value;
                  }
                  onChange(next);
                }}
                placeholder={
                  state.configured
                    ? "Stored securely on this device"
                    : state.inherited
                      ? `Configured in ${inheritedLabel}`
                      : field.placeholder
                }
                spellCheck={false}
                type={field.kind === "secret" ? "password" : "url"}
                value={state.localValue}
              />
            </div>
            <p
              className={cn(
                "text-xs",
                secureStorageUnavailable && field.kind === "secret"
                  ? "text-destructive"
                  : state.valid || !state.hasLocalValue
                    ? "text-muted-foreground"
                    : "text-destructive",
              )}
              id={helperId}
            >
              {secureStorageUnavailable && field.kind === "secret"
                ? "Secure credential storage is unavailable. Unlock it or relaunch before saving this runtime."
                : !state.valid && state.hasLocalValue
                  ? validationMessage(field)
                  : field.helperText}
            </p>
          </div>
        );
      })}
    </div>
  );
}
