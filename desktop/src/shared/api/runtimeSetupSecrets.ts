import { invokeTauri } from "./tauri";

export type RuntimeSetupSecretStatus = {
  configuredEnvKeys: string[];
  inheritedEnvKeys: string[];
};

export async function getRuntimeSetupSecretStatus({
  agentPubkey,
  definitionId,
  runtimeId,
}: {
  agentPubkey?: string | null;
  definitionId?: string | null;
  runtimeId: string;
}): Promise<RuntimeSetupSecretStatus> {
  return invokeTauri<RuntimeSetupSecretStatus>(
    "get_runtime_setup_secret_status",
    {
      agentPubkey: agentPubkey || null,
      definitionId: definitionId || null,
      runtimeId,
    },
  );
}
