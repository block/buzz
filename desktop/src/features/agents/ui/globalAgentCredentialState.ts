import type { RuntimeFileConfigSubset } from "@/shared/api/tauri";
import type { AcpProviderProfile } from "@/shared/api/types";
import {
  getBakedSatisfiedEnvKeys,
  getProviderApiKeyEnvVar,
  requiredCredentialEnvKeys,
} from "@/features/agents/ui/agentConfigOptions";

export function getGlobalAgentCredentialState({
  bakedEnvKeys,
  envVars,
  provider,
  runtimeFileConfig,
  runtimeId,
  providerProfiles,
  deviceSatisfiedEnvKeys = [],
}: {
  bakedEnvKeys: readonly string[];
  envVars: Record<string, string>;
  provider: string;
  runtimeFileConfig: RuntimeFileConfigSubset | null | undefined;
  runtimeId: string;
  providerProfiles?: readonly AcpProviderProfile[];
  deviceSatisfiedEnvKeys?: readonly string[];
}) {
  const requiredEnvKeys = requiredCredentialEnvKeys(
    runtimeId,
    provider,
    providerProfiles,
  );
  const apiKeyEnvVar = getProviderApiKeyEnvVar(provider, providerProfiles);
  const bakedSatisfiedEnvKeys = getBakedSatisfiedEnvKeys(
    requiredEnvKeys,
    envVars,
    bakedEnvKeys,
  );
  const fileSatisfiedEnvKeys = requiredEnvKeys.filter(
    (key) =>
      !(key in envVars) &&
      !bakedSatisfiedEnvKeys.includes(key) &&
      (runtimeFileConfig?.satisfiedEnvKeys.includes(key) ?? false),
  );
  const displayedRequiredEnvKeys = requiredEnvKeys.filter(
    (key) =>
      !bakedSatisfiedEnvKeys.includes(key) &&
      !fileSatisfiedEnvKeys.includes(key),
  );
  const advancedRequiredEnvKeys = displayedRequiredEnvKeys.filter(
    (key) => key !== apiKeyEnvVar,
  );
  const advancedFileSatisfiedEnvKeys = fileSatisfiedEnvKeys.filter(
    (key) => key !== apiKeyEnvVar,
  );
  const apiKeyValue = apiKeyEnvVar ? (envVars[apiKeyEnvVar] ?? "") : "";
  const apiKeyFileSatisfied =
    apiKeyEnvVar !== null && fileSatisfiedEnvKeys.includes(apiKeyEnvVar);
  const apiKeyInherited =
    apiKeyEnvVar !== null &&
    apiKeyValue.length === 0 &&
    (bakedSatisfiedEnvKeys.includes(apiKeyEnvVar) || apiKeyFileSatisfied);
  const apiKeyDeviceSatisfied =
    apiKeyEnvVar !== null && deviceSatisfiedEnvKeys.includes(apiKeyEnvVar);
  const advancedCredentialMissing = advancedRequiredEnvKeys.some(
    (key) => (envVars[key] ?? "").trim().length === 0,
  );
  const apiKeyMissing =
    apiKeyEnvVar !== null &&
    !apiKeyInherited &&
    !apiKeyDeviceSatisfied &&
    apiKeyValue.trim().length === 0;

  return {
    advancedCredentialMissing,
    advancedFileSatisfiedEnvKeys,
    advancedRequiredEnvKeys,
    apiKeyEnvVar,
    apiKeyFileSatisfied,
    apiKeyInherited,
    apiKeyDeviceSatisfied,
    apiKeyValue,
    credentialsValid: !advancedCredentialMissing && !apiKeyMissing,
  };
}
