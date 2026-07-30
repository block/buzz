import * as React from "react";

import type { EnvVarsValue } from "./EnvVarsEditor";
import {
  getBakedSatisfiedEnvKeys,
  isGloballySatisfiedCredentialKey,
} from "./agentConfigOptions";

/** Canonical env key persisted through existing env_vars maps. */
export const OPENAI_COMPAT_BASE_URL_ENV_KEY = "OPENAI_COMPAT_BASE_URL";

/**
 * OpenAI-compatible custom endpoints own a structured base-URL field.
 * Plain `openai` keeps the native default (or Advanced raw env) and does not
 * surface the field — matching product scope for custom gateways only.
 */
export function providerOwnsBaseUrlField(provider: string): boolean {
  return provider.trim().toLowerCase() === "openai-compat";
}

/** Trim surrounding whitespace before validation/persistence. */
export function normalizeProviderBaseUrl(value: string): string {
  return value.trim();
}

/**
 * Blank is valid (native default / backward compatibility).
 * Non-empty values must be http(s) URLs with a hostname.
 */
export function isValidProviderBaseUrl(value: string): boolean {
  const trimmed = normalizeProviderBaseUrl(value);
  if (trimmed.length === 0) return true;
  try {
    const parsed = new URL(trimmed);
    return (
      (parsed.protocol === "http:" || parsed.protocol === "https:") &&
      parsed.hostname.length > 0
    );
  } catch {
    return false;
  }
}

export type ProviderBaseUrlFieldState = {
  /** Env key owned by the structured field, or null when not shown. */
  envKey: string | null;
  inheritedLabel: string;
  isInherited: boolean;
  /** True when a non-empty local value fails URL validation. */
  isInvalid: boolean;
  /** Inverse of isInvalid — form gates use this. */
  isValid: boolean;
  /** Local (agent/global config) value; never copies inherited secrets/values. */
  value: string;
  /** Whether the structured field should render for this provider. */
  visible: boolean;
};

/**
 * Derive structured base-URL field state using the same env layering as the
 * API-key field. Local presence of the key (including explicit empty string)
 * shadows inherited layers. Blank remains valid for backward compatibility.
 */
export function getProviderBaseUrlFieldState({
  bakedEnvKeys,
  effectiveEnvVars,
  envVars,
  fileSatisfiedEnvKeys = [],
  globalEnvVars,
  personaSatisfied = false,
  provider,
}: {
  bakedEnvKeys: readonly string[] | undefined;
  effectiveEnvVars: EnvVarsValue;
  envVars: EnvVarsValue;
  fileSatisfiedEnvKeys?: readonly string[];
  globalEnvVars: EnvVarsValue;
  personaSatisfied?: boolean;
  provider: string;
}): ProviderBaseUrlFieldState {
  if (!providerOwnsBaseUrlField(provider)) {
    return {
      envKey: null,
      inheritedLabel: "",
      isInherited: false,
      isInvalid: false,
      isValid: true,
      value: "",
      visible: false,
    };
  }

  const envKey = OPENAI_COMPAT_BASE_URL_ENV_KEY;
  const value = envVars[envKey] ?? "";
  const localOverride = envKey in envVars;
  const source =
    normalizeProviderBaseUrl(value).length > 0
      ? null
      : personaSatisfied && !localOverride
        ? "persona"
        : isGloballySatisfiedCredentialKey(
              envKey,
              globalEnvVars,
              effectiveEnvVars,
            )
          ? "global"
          : getBakedSatisfiedEnvKeys([envKey], effectiveEnvVars, bakedEnvKeys)
                .length > 0
            ? "build"
            : !localOverride &&
                !(envKey in effectiveEnvVars) &&
                fileSatisfiedEnvKeys.includes(envKey)
              ? "file"
              : null;

  const inheritedLabel =
    source === "persona"
      ? "Inherited from agent profile"
      : source === "global"
        ? "Inherited from global config"
        : source === "build"
          ? "Inherited from build"
          : source === "file"
            ? "Set in runtime config"
            : "";

  const isValid = isValidProviderBaseUrl(value);

  return {
    envKey,
    inheritedLabel,
    isInherited: source !== null,
    isInvalid: !isValid,
    isValid,
    value,
    visible: true,
  };
}

export function useProviderBaseUrlFieldState({
  bakedEnvKeys,
  effectiveEnvVars,
  envVars,
  fileSatisfiedEnvKeys,
  globalEnvVars,
  personaSatisfied,
  provider,
}: {
  bakedEnvKeys: readonly string[] | undefined;
  effectiveEnvVars: EnvVarsValue;
  envVars: EnvVarsValue;
  fileSatisfiedEnvKeys?: readonly string[];
  globalEnvVars: EnvVarsValue;
  personaSatisfied?: boolean;
  provider: string;
}): ProviderBaseUrlFieldState {
  return React.useMemo(
    () =>
      getProviderBaseUrlFieldState({
        bakedEnvKeys,
        effectiveEnvVars,
        envVars,
        fileSatisfiedEnvKeys,
        globalEnvVars,
        personaSatisfied,
        provider,
      }),
    [
      bakedEnvKeys,
      effectiveEnvVars,
      envVars,
      fileSatisfiedEnvKeys,
      globalEnvVars,
      personaSatisfied,
      provider,
    ],
  );
}
