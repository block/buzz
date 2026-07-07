import * as React from "react";

import {
  useBakedBuildEnvKeysQuery,
  useRuntimeFileConfigQuery,
} from "@/features/agents/hooks";

import {
  getBakedSatisfiedEnvKeys,
  requiredCredentialEnvKeys,
  runtimeSupportsLlmProviderSelection,
} from "./personaDialogPickers";
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
 * Compute the runtime/provider-required credential state and keep the Advanced
 * section's required-credential row visible when a key is newly missing.
 *
 * All keys are derived from the PROSPECTIVE post-submit runtime (not the
 * current dropdown). On an inherit transition (claude→buzz-agent or the
 * reverse) the current dropdown would suppress the provider to "" and falsely
 * unblock Save; using the prospective id keeps the gate honest about what will
 * actually be saved.
 *
 * The `EnvVarsEditor` (and its amber required-key row) lives inside the
 * collapsed-by-default Advanced section, so a provider change that newly
 * requires a key would otherwise leave the row unmounted while Save is disabled
 * with no on-screen reason. This hook auto-expands Advanced on the
 * missing→present-requirement transition, so the user can still collapse it
 * again once the key is filled.
 */
export function useRequiredCredentialState(params: {
  open: boolean;
  prospectiveRuntimeId: string;
  provider: string;
  envVars: Record<string, string>;
  setShowAdvancedFields: React.Dispatch<React.SetStateAction<boolean>>;
}): RequiredCredentialState {
  const {
    open,
    prospectiveRuntimeId,
    provider,
    envVars,
    setShowAdvancedFields,
  } = params;

  const providerForRequiredKeys = runtimeSupportsLlmProviderSelection(
    prospectiveRuntimeId,
  )
    ? provider
    : "";

  const { data: runtimeFileConfig } = useRuntimeFileConfigQuery(
    prospectiveRuntimeId,
    { enabled: open },
  );

  const { data: bakedEnvKeys, isFetched: bakedEnvKeysFetched } =
    useBakedBuildEnvKeysQuery({ enabled: open });

  // All required keys for this runtime + provider combination.
  const allRequiredKeys = React.useMemo(
    () =>
      requiredCredentialEnvKeys(prospectiveRuntimeId, providerForRequiredKeys),
    [prospectiveRuntimeId, providerForRequiredKeys],
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
        (envVars[key] ?? "").length === 0 &&
        !bakedSatisfiedKeys.includes(key) &&
        runtimeFileConfig.satisfiedEnvKeys.includes(key),
    );
  }, [runtimeFileConfig, allRequiredKeys, envVars, bakedSatisfiedKeys]);

  const requiredEnvKeys = React.useMemo(
    () =>
      allRequiredKeys.filter(
        (key) =>
          !bakedSatisfiedKeys.includes(key) &&
          !fileSatisfiedEnvKeys.includes(key),
      ),
    [allRequiredKeys, bakedSatisfiedKeys, fileSatisfiedEnvKeys],
  );

  const requiredEnvKeyMissing = React.useMemo(
    () => hasMissingRequiredEnvKey(requiredEnvKeys, envVars),
    [requiredEnvKeys, envVars],
  );

  // Auto-expand Advanced on the missing→present-requirement transition only.
  // Wait for the baked-keys query to settle before firing: on first dialog open
  // the query is still in-flight so bakedEnvKeys is undefined, which transiently
  // marks baked-covered keys as missing. An errored query still counts as settled
  // (fail-closed for badge/save purposes, but no premature expand).
  const previousMissing = React.useRef(false);
  React.useEffect(() => {
    if (!open) {
      previousMissing.current = false;
      return;
    }
    if (
      requiredEnvKeyMissing &&
      !previousMissing.current &&
      bakedEnvKeysFetched
    ) {
      setShowAdvancedFields(true);
    }
    previousMissing.current = requiredEnvKeyMissing;
  }, [open, requiredEnvKeyMissing, bakedEnvKeysFetched, setShowAdvancedFields]);

  return { requiredEnvKeys, fileSatisfiedEnvKeys, requiredEnvKeyMissing };
}
