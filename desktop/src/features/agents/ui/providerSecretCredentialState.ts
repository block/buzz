import type { useProviderSecret } from "./useProviderSecret";

export type ProviderSecretState = ReturnType<typeof useProviderSecret>;

export function providerSecretDeviceField(secret: ProviderSecretState) {
  if (!secret.credential) return undefined;
  return {
    clear: secret.clear,
    configured: secret.configured,
    error: secret.error,
    failedRestartCount: secret.failedRestartCount,
    isPending: secret.isPending,
    restartedCount: secret.restartedCount,
    set: secret.set,
    source: secret.source,
  };
}

export function providerSecretSatisfiedEnvKeys(
  secret: ProviderSecretState,
): readonly string[] {
  return secret.configured && secret.credential ? [secret.credential.env] : [];
}

export function applyProviderSecretToLocalGate(
  gate: {
    satisfied: boolean;
    requiredEnvKeys: readonly string[];
    missingEnvKeys: readonly string[];
    missingNormalizedFields: readonly unknown[];
  },
  secret: ProviderSecretState,
): { requiredEnvKeys: string[]; satisfied: boolean } {
  const credentialEnv = secret.configured ? secret.credential?.env : undefined;
  return {
    requiredEnvKeys: gate.requiredEnvKeys.filter(
      (key) => key !== credentialEnv,
    ),
    satisfied:
      gate.satisfied ||
      (gate.missingNormalizedFields.length === 0 &&
        gate.missingEnvKeys.every((key) => key === credentialEnv)),
  };
}
