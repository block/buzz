import * as React from "react";
import type { RuntimeSetupField } from "@/shared/api/types";

import {
  useBakedBuildEnvKeysQuery,
  useRuntimeFileConfigQuery,
} from "@/features/agents/hooks";

import {
  getBakedSatisfiedEnvKeys,
  isGloballySatisfiedCredentialKey,
  isRuntimeSetupFieldValueValid,
  requiredCredentialEnvKeys,
  runtimeSupportsLlmProviderSelection,
} from "./agentConfigOptions";
import { hasMissingRequiredEnvKey } from "./personaRuntimeModel";

/** Derived required-credential state for the Edit Agent dialog's Advanced section. */
export interface RequiredCredentialState {
  /** Required env keys still unset and not satisfied by the runtime file config. */
  requiredEnvKeys: string[];
  /** Required keys already satisfied by the runtime file config (shown as info rows). */
  fileSatisfiedEnvKeys: string[];
  /** Whether any required env key is still missing (blocks Save). */
  requiredEnvKeyMissing: boolean;
}

/**
 * Compute runtime/provider-required credential state for the Edit Agent dialog.
 *
 * All keys are derived from the PROSPECTIVE post-submit runtime (not the
 * current dropdown). On an inherit transition (claude→buzz-agent or the
 * reverse) the current dropdown would suppress the provider to "" and falsely
 * unblock Save; using the prospective id keeps the gate honest about what will
 * actually be saved.
 *
 * The caller owns the visibility policy: top-level API-key fields are already
 * visible, while non-secret keys remain available under user-controlled
 * Advanced disclosure.
 *
 * `globalProvider` is used as the fallback when the per-agent provider is
 * empty — without it, a global-provider-only config produces no required keys
 * in the agent-instance dialogs even though the effective provider demands one.
 *
 * `globalEnvVars` is used to satisfy credential keys at the global layer:
 * a key present in global env is NOT required (no amber row, save not blocked).
 * This mirrors the same agent→global→file precedence used by
 * `computeLocalModeGate`. Without it, a key covered only by global env still
 * marks `requiredEnvKeyMissing=true` and silently disables Save.
 */
export function useRequiredCredentialState(params: {
  open: boolean;
  prospectiveRuntimeId: string;
  setupFields?: readonly RuntimeSetupField[];
  setupSatisfiedEnvKeys?: readonly string[];
  providerEnvVar?: string | null;
  provider: string;
  /** Global provider default; used as fallback when per-agent provider is empty. */
  globalProvider?: string;
  envVars: Record<string, string>;
  /** Global config env vars; keys satisfied here are excluded from required
   *  rows and do not block Save — mirrors the agent→global→file precedence. */
  globalEnvVars?: Record<string, string>;
  /** Persona env vars; keys satisfied here are excluded from required rows
   *  (mirrors backend readiness.rs which layers live_persona_env under record
   *  env). An explicit local empty shadows the persona value. */
  personaEnvVars?: Record<string, string>;
}): RequiredCredentialState {
  const {
    open,
    prospectiveRuntimeId,
    setupFields = [],
    setupSatisfiedEnvKeys = [],
    providerEnvVar,
    provider,
    globalProvider = "",
    envVars,
    globalEnvVars = {},
    personaEnvVars = {},
  } = params;

  const providerForRequiredKeys = runtimeSupportsLlmProviderSelection(
    providerEnvVar,
  )
    ? provider.trim() || globalProvider.trim()
    : "";

  const { data: runtimeFileConfig } = useRuntimeFileConfigQuery(
    prospectiveRuntimeId,
    { enabled: open },
  );

  const { data: bakedEnvKeys } = useBakedBuildEnvKeysQuery({ enabled: open });

  // All required keys for this runtime + provider combination.
  const allRequiredKeys = React.useMemo(
    () =>
      requiredCredentialEnvKeys(
        prospectiveRuntimeId,
        providerForRequiredKeys,
        setupFields,
      ),
    [prospectiveRuntimeId, providerForRequiredKeys, setupFields],
  );
  const setupFieldsByKey = React.useMemo(
    () => new Map(setupFields.map((field) => [field.envKey, field])),
    [setupFields],
  );
  const setupSatisfiedSet = React.useMemo(
    () => new Set(setupSatisfiedEnvKeys),
    [setupSatisfiedEnvKeys],
  );

  // Keys covered by the baked build env — silenced, produce no info row.
  const bakedSatisfiedKeys = React.useMemo(
    () => getBakedSatisfiedEnvKeys(allRequiredKeys, envVars, bakedEnvKeys),
    [allRequiredKeys, envVars, bakedEnvKeys],
  );

  const fileSatisfiedEnvKeys = React.useMemo(() => {
    if (!runtimeFileConfig) return [] as string[];
    return allRequiredKeys.filter(
      (key) =>
        !setupFieldsByKey.has(key) &&
        !(key in envVars) &&
        !bakedSatisfiedKeys.includes(key) &&
        runtimeFileConfig.satisfiedEnvKeys.includes(key),
    );
  }, [
    runtimeFileConfig,
    allRequiredKeys,
    envVars,
    bakedSatisfiedKeys,
    setupFieldsByKey,
  ]);

  const requiredEnvKeys = React.useMemo(
    () =>
      allRequiredKeys.filter((key) => {
        const setupField = setupFieldsByKey.get(key);
        if (setupField) {
          if (!(key in envVars) && setupSatisfiedSet.has(key)) {
            return false;
          }
          return !isGloballySatisfiedCredentialKey(
            key,
            personaEnvVars,
            envVars,
            setupField,
          );
        }
        return (
          !bakedSatisfiedKeys.includes(key) &&
          !fileSatisfiedEnvKeys.includes(key) &&
          !isGloballySatisfiedCredentialKey(key, globalEnvVars, envVars) &&
          !isGloballySatisfiedCredentialKey(key, personaEnvVars, envVars)
        );
      }),
    [
      allRequiredKeys,
      bakedSatisfiedKeys,
      fileSatisfiedEnvKeys,
      globalEnvVars,
      personaEnvVars,
      envVars,
      setupFieldsByKey,
      setupSatisfiedSet,
    ],
  );

  const requiredEnvKeyMissing = React.useMemo(
    () =>
      requiredEnvKeys.some((key) => {
        const setupField = setupFieldsByKey.get(key);
        return setupField
          ? !isRuntimeSetupFieldValueValid(setupField, envVars[key])
          : hasMissingRequiredEnvKey([key], envVars);
      }),
    [requiredEnvKeys, envVars, setupFieldsByKey],
  );

  return {
    requiredEnvKeys,
    fileSatisfiedEnvKeys,
    requiredEnvKeyMissing,
  };
}
