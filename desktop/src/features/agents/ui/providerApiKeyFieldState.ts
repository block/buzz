import * as React from "react";

import type { EnvVarsValue } from "./EnvVarsEditor";
import {
  getBakedSatisfiedEnvKeys,
  getProviderApiKeyEnvVar,
  isGloballySatisfiedCredentialKey,
} from "./agentConfigOptions";

export type ProviderApiKeyFieldState = {
  advancedRequiredEnvKeys: readonly string[];
  inheritedLabel: string;
  isInherited: boolean;
  isRequired: boolean;
  secretEnvVar: string | null;
  value: string;
};

/**
 * Derive the top-level API-key field from the same env snapshots that drive
 * readiness. `envVars` is the raw local record, while `effectiveEnvVars` may
 * include a persona snapshot during an instance inherit transition.
 */
export function getProviderApiKeyFieldState({
  bakedEnvKeys,
  effectiveEnvVars,
  envVars,
  fileSatisfiedEnvKeys = [],
  globalEnvVars,
  personaSatisfied = false,
  provider,
  requiredEnvKeys,
}: {
  bakedEnvKeys: readonly string[] | undefined;
  effectiveEnvVars: EnvVarsValue;
  envVars: EnvVarsValue;
  fileSatisfiedEnvKeys?: readonly string[];
  globalEnvVars: EnvVarsValue;
  personaSatisfied?: boolean;
  provider: string;
  requiredEnvKeys: readonly string[];
}): ProviderApiKeyFieldState {
  const secretEnvVar = getProviderApiKeyEnvVar(provider);
  const advancedRequiredEnvKeys = secretEnvVar
    ? requiredEnvKeys.filter((key) => key !== secretEnvVar)
    : requiredEnvKeys;
  if (!secretEnvVar) {
    return {
      advancedRequiredEnvKeys,
      inheritedLabel: "",
      isInherited: false,
      isRequired: false,
      secretEnvVar: null,
      value: "",
    };
  }

  const value = envVars[secretEnvVar] ?? "";
  const localOverride = secretEnvVar in envVars;
  const source =
    value.length > 0
      ? null
      : personaSatisfied && !localOverride
        ? "persona"
        : isGloballySatisfiedCredentialKey(
              secretEnvVar,
              globalEnvVars,
              effectiveEnvVars,
            )
          ? "global"
          : getBakedSatisfiedEnvKeys(
                [secretEnvVar],
                effectiveEnvVars,
                bakedEnvKeys,
              ).length > 0
            ? "build"
            : !(secretEnvVar in effectiveEnvVars) &&
                fileSatisfiedEnvKeys.includes(secretEnvVar)
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

  return {
    advancedRequiredEnvKeys,
    inheritedLabel,
    isInherited: source !== null,
    isRequired: source === null && value.length === 0,
    secretEnvVar,
    value,
  };
}

export function useProviderApiKeyFieldState({
  bakedEnvKeys,
  effectiveEnvVars,
  envVars,
  fileSatisfiedEnvKeys,
  globalEnvVars,
  open,
  personaSatisfied,
  provider,
  requiredEnvKeys,
  satisfactionSettled = true,
  setShowAdvancedFields,
}: {
  bakedEnvKeys: readonly string[] | undefined;
  effectiveEnvVars: EnvVarsValue;
  envVars: EnvVarsValue;
  fileSatisfiedEnvKeys?: readonly string[];
  globalEnvVars: EnvVarsValue;
  open: boolean;
  personaSatisfied?: boolean;
  provider: string;
  requiredEnvKeys: readonly string[];
  satisfactionSettled?: boolean;
  setShowAdvancedFields: React.Dispatch<React.SetStateAction<boolean>>;
}): ProviderApiKeyFieldState {
  const fieldState = React.useMemo(
    () =>
      getProviderApiKeyFieldState({
        bakedEnvKeys,
        effectiveEnvVars,
        envVars,
        fileSatisfiedEnvKeys,
        globalEnvVars,
        personaSatisfied,
        provider,
        requiredEnvKeys,
      }),
    [
      bakedEnvKeys,
      effectiveEnvVars,
      envVars,
      fileSatisfiedEnvKeys,
      globalEnvVars,
      personaSatisfied,
      provider,
      requiredEnvKeys,
    ],
  );
  const hasAutoOpenedAdvancedRef = React.useRef(false);
  React.useEffect(() => {
    if (!open) {
      hasAutoOpenedAdvancedRef.current = false;
      return;
    }
    if (
      satisfactionSettled &&
      fieldState.advancedRequiredEnvKeys.length > 0 &&
      !hasAutoOpenedAdvancedRef.current
    ) {
      hasAutoOpenedAdvancedRef.current = true;
      setShowAdvancedFields(true);
    }
  }, [
    fieldState.advancedRequiredEnvKeys.length,
    open,
    satisfactionSettled,
    setShowAdvancedFields,
  ]);

  return fieldState;
}
