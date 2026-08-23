import { invokeTauri } from "./tauriInvoke";

export type ProviderSecretStatus = {
  providerId: string;
  credentialId: string;
  configured: boolean;
  source: "environment" | "keyring" | "missing" | "unavailable";
  restartedCount: number;
  failedRestartCount: number;
};

/** Return only device-credential presence/source metadata, never its value. */
export function getProviderSecretStatus(
  providerId: string,
): Promise<ProviderSecretStatus> {
  return invokeTauri("get_provider_secret_status", { providerId });
}

export function setProviderSecret(
  providerId: string,
  value: string,
): Promise<ProviderSecretStatus> {
  return invokeTauri("set_provider_secret", { providerId, value });
}

export function clearProviderSecret(
  providerId: string,
): Promise<ProviderSecretStatus> {
  return invokeTauri("clear_provider_secret", { providerId });
}
