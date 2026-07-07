import * as React from "react";

import { useRuntimeFileConfigQuery } from "@/features/agents/hooks";

import {
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

  const fileSatisfiedEnvKeys = React.useMemo(() => {
    if (!runtimeFileConfig) return [] as string[];
    return requiredCredentialEnvKeys(
      prospectiveRuntimeId,
      providerForRequiredKeys,
    ).filter(
      (key) =>
        (envVars[key] ?? "").length === 0 &&
        runtimeFileConfig.satisfiedEnvKeys.includes(key),
    );
  }, [
    runtimeFileConfig,
    prospectiveRuntimeId,
    providerForRequiredKeys,
    envVars,
  ]);

  const requiredEnvKeys = React.useMemo(
    () =>
      requiredCredentialEnvKeys(
        prospectiveRuntimeId,
        providerForRequiredKeys,
      ).filter((key) => !fileSatisfiedEnvKeys.includes(key)),
    [prospectiveRuntimeId, providerForRequiredKeys, fileSatisfiedEnvKeys],
  );

  const requiredEnvKeyMissing = React.useMemo(
    () => hasMissingRequiredEnvKey(requiredEnvKeys, envVars),
    [requiredEnvKeys, envVars],
  );

  // Auto-expand Advanced on the missing→present-requirement transition only.
  const previousMissing = React.useRef(false);
  React.useEffect(() => {
    if (!open) {
      previousMissing.current = false;
      return;
    }
    if (requiredEnvKeyMissing && !previousMissing.current) {
      setShowAdvancedFields(true);
    }
    previousMissing.current = requiredEnvKeyMissing;
  }, [open, requiredEnvKeyMissing, setShowAdvancedFields]);

  return { requiredEnvKeys, fileSatisfiedEnvKeys, requiredEnvKeyMissing };
}
