import type { Profile, UpdateProfileInput } from "@/shared/api/types";
import { invokeTauri } from "@/shared/api/tauri";

type RawExternalAgentIdentityStatus = {
  linked: boolean;
};

type RawProfile = {
  pubkey: string;
  display_name: string | null;
  avatar_url: string | null;
  about: string | null;
  nip05_handle: string | null;
  owner_pubkey: string | null;
  has_profile_event?: boolean;
};

function fromRawProfile(profile: RawProfile): Profile {
  return {
    pubkey: profile.pubkey,
    displayName: profile.display_name,
    avatarUrl: profile.avatar_url,
    about: profile.about,
    nip05Handle: profile.nip05_handle,
    ownerPubkey: profile.owner_pubkey,
    hasProfileEvent: profile.has_profile_event ?? false,
  };
}

export async function getExternalAgentIdentityStatus(
  pubkey: string,
): Promise<RawExternalAgentIdentityStatus> {
  return invokeTauri("get_external_agent_identity_status", { pubkey });
}

export async function linkExternalAgentIdentity(
  pubkey: string,
  nsec: string,
): Promise<RawExternalAgentIdentityStatus> {
  return invokeTauri("link_external_agent_identity", { nsec, pubkey });
}

export async function updateExternalAgentProfile(
  pubkey: string,
  input: Pick<UpdateProfileInput, "displayName" | "avatarUrl" | "about">,
): Promise<Profile> {
  const profile = await invokeTauri<RawProfile>(
    "update_external_agent_profile",
    {
      ...input,
      pubkey,
    },
  );
  return fromRawProfile(profile);
}
