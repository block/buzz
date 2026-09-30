import { invokeTauri } from "@/shared/api/tauri";

export async function applyCommunity(
  relayUrl: string,
  nsec?: string,
  token?: string,
  reposDir?: string,
  agentManagedProfiles?: boolean,
): Promise<void> {
  await invokeTauri("apply_workspace", {
    relayUrl,
    nsec: nsec ?? null,
    token: token ?? null,
    reposDir: reposDir ?? null,
    agentManagedProfiles: agentManagedProfiles ?? false,
  });
}

/** Stop any in-flight launch restore from spawning after a community removal. */
export const invalidateLaunchRestore = () =>
  invokeTauri<void>("invalidate_launch_restore");

export const setAgentManagedProfiles = (enabled: boolean) =>
  invokeTauri("set_agent_managed_profiles", { enabled });

/** Refresh source trust without applying/resetting the active workspace. */
export const setAgentAvatarCommunities = (relayUrls: string[]) =>
  invokeTauri<void>("set_agent_avatar_communities", { relayUrls });
