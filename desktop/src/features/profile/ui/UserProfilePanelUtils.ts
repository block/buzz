import * as React from "react";
import type {
  AgentPersona,
  Channel,
  ManagedAgent,
  Profile,
  RelayAgent,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type ProfileChannelLink = {
  id: string;
  name: string;
};

export type ProfilePanelView =
  | "summary"
  | "info"
  | "settings"
  | "diagnostics"
  | "model"
  | "instructions"
  | "memories"
  | "channels"
  | "logs";

export const PROFILE_PANEL_VIEW_TITLES: Record<ProfilePanelView, string> = {
  summary: "Profile",
  info: "Agent info",
  settings: "Agent settings",
  diagnostics: "Diagnostics",
  model: "Model",
  instructions: "Agent instruction",
  memories: "Memories",
  channels: "Channels",
  logs: "Harness log",
};

export type UserProfilePanelProps = {
  canResetWidth?: boolean;
  currentPubkey?: string;
  isSinglePanelView?: boolean;
  layout?: "standalone" | "split";
  onClose: () => void;
  onOpenDm?: (pubkeys: string[]) => void;
  onResetWidth?: () => void;
  onResizeStart?: (event: React.PointerEvent<HTMLButtonElement>) => void;
  onViewChange?: (
    view: ProfilePanelView,
    options?: { replace?: boolean },
  ) => void;
  persona?: AgentPersona;
  pubkey?: string;
  splitPaneClamp?: boolean;
  view?: ProfilePanelView;
  widthPx: number;
};

export function truncatePubkey(pubkey: string) {
  if (pubkey.length <= 16) {
    return pubkey;
  }

  return `${pubkey.slice(0, 8)}…${pubkey.slice(-8)}`;
}

export function deriveProfileChannels(
  pubkeyLower: string,
  relayAgent: RelayAgent | undefined,
  managedAgent: ManagedAgent | undefined,
  channels: Channel[] | undefined,
): ProfileChannelLink[] {
  const links = new Map<string, ProfileChannelLink>();
  const channelsByName = new Map(
    channels?.map((channel) => [channel.name, channel]) ?? [],
  );

  relayAgent?.channels.forEach((name, index) => {
    const channel = channelsByName.get(name);
    const id = relayAgent.channelIds[index] ?? channel?.id ?? name;
    links.set(id, { id, name });
  });

  if (managedAgent && channels) {
    for (const channel of channels) {
      const isMember = channel.memberPubkeys.some(
        (memberPubkey) => memberPubkey.toLowerCase() === pubkeyLower,
      );
      if (isMember) {
        links.set(channel.id, { id: channel.id, name: channel.name });
      }
    }
  }

  return [...links.values()].sort((left, right) =>
    left.name.localeCompare(right.name),
  );
}

export function getRelayAgentChannelIds(
  relayAgents: readonly RelayAgent[] | undefined,
  agentPubkey: string,
): string[] {
  const normalized = normalizePubkey(agentPubkey);
  const agent = (relayAgents ?? []).find(
    (candidate) => normalizePubkey(candidate.pubkey) === normalized,
  );
  return agent?.channelIds ?? [];
}

export function buildPersonaDraftProfile(persona: AgentPersona): Profile {
  return {
    pubkey: "",
    displayName: persona.displayName,
    avatarUrl: persona.avatarUrl,
    about: null,
    nip05Handle: null,
  };
}

export function resolveProfileDisplayName({
  persona,
  profile,
  pubkey,
}: {
  persona: AgentPersona | undefined;
  profile: Profile | undefined;
  pubkey: string | null;
}) {
  return (
    profile?.displayName ??
    persona?.displayName ??
    (pubkey ? truncatePubkey(pubkey) : "Agent")
  );
}

export function resolveOwnerHandle(
  profile: Profile | undefined,
  currentPubkey: string | undefined,
) {
  if (currentPubkey === undefined) {
    return null;
  }

  return (
    profile?.nip05Handle?.trim() ||
    profile?.displayName?.trim() ||
    truncatePubkey(currentPubkey)
  );
}

export function resolveAgentInstruction(
  managedAgent: ManagedAgent | undefined,
  persona: AgentPersona | undefined,
) {
  return (
    managedAgent?.systemPrompt?.trim() || persona?.systemPrompt.trim() || null
  );
}

export function useRetainedPersona(
  sourcePersona: AgentPersona | undefined,
  profileIdentityKey: string,
) {
  const [retainedPersona, setRetainedPersona] = React.useState<{
    key: string;
    persona: AgentPersona;
  } | null>(null);

  React.useEffect(() => {
    if (!sourcePersona) return;
    setRetainedPersona({ key: profileIdentityKey, persona: sourcePersona });
  }, [profileIdentityKey, sourcePersona]);

  return (
    sourcePersona ??
    (retainedPersona?.key === profileIdentityKey
      ? retainedPersona.persona
      : undefined)
  );
}
