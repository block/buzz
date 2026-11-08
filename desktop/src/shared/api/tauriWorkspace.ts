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

/** Refuse local agent pairs on a removed community's relay, including starts already in flight. */
export const removeCommunityRelay = (relayUrl: string) =>
  invokeTauri<void>("remove_community_relay", { relayUrl });

/** Admit local agent pairs on a relay again after its community is re-added. */
export const readdCommunityRelay = (relayUrl: string) =>
  invokeTauri<void>("readd_community_relay", { relayUrl });

export const setAgentManagedProfiles = (enabled: boolean) =>
  invokeTauri("set_agent_managed_profiles", { enabled });

/** Refresh source trust without applying/resetting the active workspace. */
export const setAgentAvatarCommunities = (relayUrls: string[]) =>
  invokeTauri<void>("set_agent_avatar_communities", { relayUrls });
