import type { RuntimeFileConfigSubset } from "@/shared/api/tauri";
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
}: {
  bakedEnvKeys: readonly string[];
  envVars: Record<string, string>;
  provider: string;
  runtimeFileConfig: RuntimeFileConfigSubset | null | undefined;
  runtimeId: string;
}) {
  const requiredEnvKeys = requiredCredentialEnvKeys(runtimeId, provider);
  const apiKeyEnvVar = getProviderApiKeyEnvVar(provider);
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
  const advancedCredentialMissing = advancedRequiredEnvKeys.some(
    (key) => (envVars[key] ?? "").trim().length === 0,
  );
  const apiKeyMissing =
    apiKeyEnvVar !== null &&
    !apiKeyInherited &&
    apiKeyValue.trim().length === 0;

  return {
    advancedCredentialMissing,
    advancedFileSatisfiedEnvKeys,
    advancedRequiredEnvKeys,
    apiKeyEnvVar,
    apiKeyFileSatisfied,
    apiKeyInherited,
    apiKeyValue,
    credentialsValid: !advancedCredentialMissing && !apiKeyMissing,
  };
}
