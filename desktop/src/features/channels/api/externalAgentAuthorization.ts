import { invoke as tauriInvoke } from "@tauri-apps/api/core";

export async function authorizeExternalAgent(
  channelId: string,
  agentPubkey: string,
): Promise<string> {
  try {
    return await tauriInvoke<string>("authorize_external_agent", {
      channelId,
      agentPubkey,
    });
  } catch (error) {
    throw error instanceof Error ? error : new Error(String(error));
  }
}
