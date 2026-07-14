import * as React from "react";

import {
  useBakedBuildEnvKeysQuery,
  useRuntimeFileConfigQuery,
} from "@/features/agents/hooks";

import {
  getBakedSatisfiedEnvKeys,
  isGloballySatisfiedCredentialKey,
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
 * Compute runtime/provider-required credential state for the Edit Agent dialog.
 *
 * All keys are derived from the PROSPECTIVE post-submit runtime (not the
 * current dropdown). On an inherit transition (claude→buzz-agent or the
 * reverse) the current dropdown would suppress the provider to "" and falsely
 * unblock Save; using the prospective id keeps the gate honest about what will
 * actually be saved.
 *
 * The caller owns the visibility policy: top-level API-key fields are already
 * visible, while non-secret Advanced-only keys can open that section.
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
  provider: string;
  /** Global provider default; used as fallback when per-agent provider is empty. */
  globalProvider?: string;
  envVars: Record<string, string>;
  /** Global config env vars; keys satisfied here are excluded from required
   *  rows and do not block Save — mirrors the agent→global→file precedence. */
  globalEnvVars?: Record<string, string>;
}): RequiredCredentialState {
  const {
    open,
    prospectiveRuntimeId,
    provider,
    globalProvider = "",
    envVars,
    globalEnvVars = {},
  } = params;

  const providerForRequiredKeys = runtimeSupportsLlmProviderSelection(
    prospectiveRuntimeId,
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
        !(key in envVars) &&
        !bakedSatisfiedKeys.includes(key) &&
        runtimeFileConfig.satisfiedEnvKeys.includes(key),
    );
  }, [runtimeFileConfig, allRequiredKeys, envVars, bakedSatisfiedKeys]);

  const requiredEnvKeys = React.useMemo(
    () =>
      allRequiredKeys.filter(
        (key) =>
          !bakedSatisfiedKeys.includes(key) &&
          !fileSatisfiedEnvKeys.includes(key) &&
          // isGloballySatisfiedCredentialKey returns true when global has the key
          // AND agent-local has NOT explicitly shadowed it with "" — same semantics
          // as computeLocalModeGate, preventing create/edit gate drift.
          !isGloballySatisfiedCredentialKey(key, globalEnvVars, envVars),
      ),
    [
      allRequiredKeys,
      bakedSatisfiedKeys,
      fileSatisfiedEnvKeys,
      globalEnvVars,
      envVars,
    ],
  );

  const requiredEnvKeyMissing = React.useMemo(
    () => hasMissingRequiredEnvKey(requiredEnvKeys, envVars),
    [requiredEnvKeys, envVars],
  );

  return { requiredEnvKeys, fileSatisfiedEnvKeys, requiredEnvKeyMissing };
}
