import { invoke, isTauri } from "@tauri-apps/api/core";

/** Opens a channel-scoped agent activity feed in its native companion window. */
export async function openAgentActivityWindow(
  communityId: string,
  channelId: string,
  pubkey: string,
): Promise<boolean> {
  if (!isTauri()) return false;
  return invoke<boolean>("open_agent_activity_window", {
    communityId,
    channelId,
    pubkey,
  });
}
