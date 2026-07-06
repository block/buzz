import * as React from "react";
import type {
  AcpRuntimeCatalogEntry,
  AgentPersona,
  Channel,
  ManagedAgent,
  Profile,
  RelayAgent,
  UpdateManagedAgentInput,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { truncatePubkey } from "@/features/profile/lib/identity";

export { truncatePubkey };

export type ProfileChannelLink = {
  id: string;
  name: string;
};

export type ProfilePanelView =
  | "summary"
  | "instructions"
  | "info"
  | "configuration"
  | "diagnostics"
  | "memories"
  | "channels"
  | "logs";

export type ProfilePanelTab = "info" | "runtime" | "channels" | "memories";

export const PROFILE_PANEL_VIEW_TITLES: Record<ProfilePanelView, string> = {
  summary: "Profile",
  instructions: "Instructions",
  info: "Agent info",
  configuration: "Runtime",
  diagnostics: "Harness Log",
  memories: "Memories",
  channels: "Channels",
  logs: "Harness Log",
};

const PROFILE_PANEL_VIEWS = new Set<ProfilePanelView>(
  Object.keys(PROFILE_PANEL_VIEW_TITLES) as ProfilePanelView[],
);

const PROFILE_PANEL_TABS = new Set<ProfilePanelTab>([
  "info",
  "runtime",
  "channels",
  "memories",
]);

const LEGACY_PROFILE_PANEL_VIEW_ALIASES: Record<string, ProfilePanelView> = {
  model: "configuration",
  settings: "configuration",
};

export function parseProfilePanelView(value: unknown): ProfilePanelView | null {
  if (typeof value !== "string") {
    return null;
  }

  if (PROFILE_PANEL_VIEWS.has(value as ProfilePanelView)) {
    return value as ProfilePanelView;
  }

  return LEGACY_PROFILE_PANEL_VIEW_ALIASES[value] ?? null;
}

export function profilePanelViewFromSearch(value: unknown): ProfilePanelView {
  return parseProfilePanelView(value) ?? "summary";
}

export function parseProfilePanelTab(value: unknown): ProfilePanelTab | null {
  if (typeof value !== "string") {
    return null;
  }

  if (PROFILE_PANEL_TABS.has(value as ProfilePanelTab)) {
    return value as ProfilePanelTab;
  }

  return null;
}

export function profilePanelTabFromSearch(value: unknown): ProfilePanelTab {
  return parseProfilePanelTab(value) ?? "info";
}

export type UserProfilePanelProps = {
  canResetWidth?: boolean;
  currentPubkey?: string;
  isSinglePanelView?: boolean;
  layout?: "standalone" | "split";
  onClose: () => void;
  onOpenDm?: (pubkeys: string[]) => Promise<void> | void;
  onOpenProfile?: (pubkey: string) => void;
  onResetWidth?: () => void;
  onResizeStart?: (event: React.PointerEvent<HTMLButtonElement>) => void;
  onTabChange?: (tab: ProfilePanelTab, options?: { replace?: boolean }) => void;
  onViewChange?: (
    view: ProfilePanelView,
    options?: { replace?: boolean },
  ) => void;
  persona?: AgentPersona;
  pubkey?: string;
  splitPaneClamp?: boolean;
  tab?: ProfilePanelTab;
  view?: ProfilePanelView;
  widthPx: number;
  transparentChrome?: boolean;
};

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
    ownerPubkey: null,
    // Draft profile synthesised from persona config — not backed by a kind:0 event.
    hasProfileEvent: false,
  };
}

export function resolvePanelProfile({
  persona,
  profile,
}: {
  managedAgent: ManagedAgent | undefined;
  persona: AgentPersona | undefined;
  profile: Profile | undefined;
}): Profile | undefined {
  const baseProfile =
    profile ?? (persona ? buildPersonaDraftProfile(persona) : undefined);
  return withProfileAvatarFallback(baseProfile, [persona?.avatarUrl]);
}

export function resolveProfileAvatarUrl(
  ...candidates: Array<string | null | undefined>
): string | null {
  for (const candidate of candidates) {
    const trimmed = candidate?.trim();
    if (trimmed) return trimmed;
  }
  return null;
}

export function withProfileAvatarFallback(
  profile: Profile | undefined,
  fallbackAvatarUrls: Array<string | null | undefined>,
): Profile | undefined {
  const profileAvatarUrl = normalizeProfileFallbackAvatarUrl(
    profile?.avatarUrl,
  );
  const avatarUrl = resolveProfileAvatarUrl(
    profileAvatarUrl,
    ...fallbackAvatarUrls.map((avatarUrl) =>
      normalizeProfileFallbackAvatarUrl(avatarUrl),
    ),
  );
  return profile && avatarUrl !== profile.avatarUrl
    ? { ...profile, avatarUrl }
    : profile;
}

function normalizeProfileFallbackAvatarUrl(
  avatarUrl: string | null | undefined,
): string | null {
  const trimmed = avatarUrl?.trim();
  if (!trimmed) return null;
  return trimmed;
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

export function personaManagedAgentUpdate(
  agent: ManagedAgent,
  persona: AgentPersona,
  options: {
    previousPersona?: AgentPersona;
    runtimes?: readonly AcpRuntimeCatalogEntry[];
  } = {},
): UpdateManagedAgentInput | null {
  if (agent.personaId !== persona.id) return null;

  const input: UpdateManagedAgentInput = { pubkey: agent.pubkey };
  let hasChanges = false;

  if (persona.displayName !== agent.name) {
    input.name = persona.displayName;
    hasChanges = true;
  }

  if (persona.systemPrompt !== (agent.systemPrompt ?? "")) {
    input.systemPrompt = persona.systemPrompt;
    hasChanges = true;
  }

  if ((persona.model ?? null) !== (agent.model ?? null)) {
    input.model = persona.model;
    hasChanges = true;
  }

  if (!stringRecordEqual(persona.envVars, agent.envVars)) {
    input.envVars = persona.envVars;
    hasChanges = true;
  }

  const runtimeChanged =
    options.previousPersona !== undefined &&
    options.previousPersona.runtime !== persona.runtime;
  const runtime = runtimeChanged
    ? options.runtimes?.find((candidate) => candidate.id === persona.runtime)
    : undefined;
  if (runtime?.command) {
    if (runtime.command !== agent.agentCommand) {
      input.agentCommand = runtime.command;
      hasChanges = true;
    }

    if (!stringArrayEqual(runtime.defaultArgs, agent.agentArgs)) {
      input.agentArgs = [...runtime.defaultArgs];
      hasChanges = true;
    }

    const mcpCommand = runtime.mcpCommand ?? "";
    if (mcpCommand !== agent.mcpCommand) {
      input.mcpCommand = mcpCommand;
      hasChanges = true;
    }
  }

  return hasChanges ? input : null;
}

function stringArrayEqual(left: readonly string[], right: readonly string[]) {
  if (left.length !== right.length) return false;

  return left.every((value, index) => value === right[index]);
}

function stringRecordEqual(
  left: Record<string, string>,
  right: Record<string, string>,
) {
  const leftKeys = Object.keys(left);
  const rightKeys = Object.keys(right);
  if (leftKeys.length !== rightKeys.length) return false;

  return leftKeys.every((key) => left[key] === right[key]);
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
