import type * as React from "react";

import type { EnvVarsValue } from "./EnvVarsEditor";
import { PersonaProviderApiKeyField } from "./PersonaProviderApiKeyField";
import { PersonaProviderBaseUrlField } from "./PersonaProviderBaseUrlField";
import type { ProviderApiKeyFieldState } from "./providerApiKeyFieldState";
import {
  OPENAI_COMPAT_BASE_URL_ENV_KEY,
  useProviderBaseUrlFieldState,
  type ProviderBaseUrlFieldState,
} from "./providerBaseUrlFieldState";

export function advancedHiddenEnvKeys(
  secretEnvVar: string | null | undefined,
  baseUrlEnvKey: string | null | undefined,
): string[] {
  return [
    ...(secretEnvVar ? [secretEnvVar] : []),
    ...(baseUrlEnvKey ? [baseUrlEnvKey] : []),
  ];
}

export function withBaseUrlEnvVar(
  envVars: EnvVarsValue,
  next: string,
): EnvVarsValue {
  return { ...envVars, [OPENAI_COMPAT_BASE_URL_ENV_KEY]: next };
}

export function isOpenAiCompatBaseUrlOwned(
  envVars: EnvVarsValue,
  inheritedEnvVars: EnvVarsValue,
): boolean {
  return (
    !(OPENAI_COMPAT_BASE_URL_ENV_KEY in envVars) &&
    (inheritedEnvVars[OPENAI_COMPAT_BASE_URL_ENV_KEY] ?? "").length > 0
  );
}

/** Compact structured API-key + base-URL fields for provider config surfaces. */
export function usePersonaProviderStructuredEnv({
  apiKey,
  apiKeyLabel,
  bakedEnvKeys,
  disabled,
  effectiveEnvVars,
  envVars,
  fileSatisfiedEnvKeys,
  globalEnvVars,
  onEnvVarsChange,
  personaSatisfied,
  provider,
  wrapperClassName,
}: {
  apiKey: ProviderApiKeyFieldState | null;
  apiKeyLabel: string;
  bakedEnvKeys: readonly string[] | undefined;
  disabled: boolean;
  effectiveEnvVars: EnvVarsValue;
  envVars: EnvVarsValue;
  fileSatisfiedEnvKeys?: readonly string[];
  globalEnvVars: EnvVarsValue;
  onEnvVarsChange: (next: EnvVarsValue) => void;
  personaSatisfied?: boolean;
  provider: string;
  wrapperClassName?: string;
}): {
  fields: React.ReactNode;
  hiddenEnvKeys: string[];
  isValid: boolean;
  baseUrl: ProviderBaseUrlFieldState;
} {
  const baseUrl = useProviderBaseUrlFieldState({
    bakedEnvKeys,
    effectiveEnvVars,
    envVars,
    fileSatisfiedEnvKeys,
    globalEnvVars,
    personaSatisfied,
    provider,
  });
  const secretEnvVar = apiKey?.secretEnvVar;
  const showApiKey = apiKey != null && secretEnvVar != null;
  const wrap = (node: React.ReactNode) =>
    wrapperClassName ? <div className={wrapperClassName}>{node}</div> : node;

  const fields =
    showApiKey || baseUrl.visible ? (
      <>
        {showApiKey && apiKey
          ? wrap(
              <PersonaProviderApiKeyField
                disabled={disabled}
                inheritedLabel={apiKey.inheritedLabel}
                isInherited={apiKey.isInherited}
                isRequired={apiKey.isRequired}
                label={apiKeyLabel}
                onValueChange={(next) => {
                  if (!secretEnvVar) return;
                  onEnvVarsChange({ ...envVars, [secretEnvVar]: next });
                }}
                value={apiKey.value}
              />,
            )
          : null}
        {baseUrl.visible
          ? wrap(
              <PersonaProviderBaseUrlField
                disabled={disabled}
                errorMessage={baseUrl.errorMessage}
                inheritedLabel={baseUrl.inheritedLabel}
                isInherited={baseUrl.isInherited}
                isInvalid={baseUrl.isInvalid}
                onValueChange={(next) => {
                  onEnvVarsChange(withBaseUrlEnvVar(envVars, next));
                }}
                value={baseUrl.value}
              />,
            )
          : null}
      </>
    ) : null;

  return {
    baseUrl,
    fields,
    hiddenEnvKeys: advancedHiddenEnvKeys(secretEnvVar, baseUrl.envKey),
    isValid: baseUrl.isValid,
  };
}

export { OPENAI_COMPAT_BASE_URL_ENV_KEY };
