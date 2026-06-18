import type {
  Channel,
  ChannelMember,
  CreateChannelInput,
  UpdateChannelInput,
} from "@/shared/api/types";

export const WELCOME_CHANNEL_NAME = "Welcome";
export const WELCOME_CHANNEL_DESCRIPTION =
  "A private channel for getting oriented in this workspace.";
export const WELCOME_CHANNEL_READY_EVENT =
  "buzz:onboarding-welcome-channel-ready";

const PENDING_WELCOME_CHANNEL_STORAGE_KEY =
  "buzz:onboarding-welcome-channel.v1";
const WELCOME_INITIAL_UNREAD_SUPPRESSION_STORAGE_KEY =
  "buzz:onboarding-welcome-initial-unread-suppression.v1";
const PENDING_WELCOME_CHANNEL_MAX_AGE_MS = 5 * 60 * 1000;
const WELCOME_CHANNEL_ENSURED_STORAGE_KEY = "buzz-welcome-channel-ensured.v2";

type WelcomeChannelClient = {
  createChannel: (input: CreateChannelInput) => Promise<Channel>;
  getChannels: () => Promise<Channel[]>;
  getChannelMembers?: (channelId: string) => Promise<ChannelMember[]>;
  updateChannel?: (input: UpdateChannelInput) => Promise<Channel>;
};

type WelcomeChannelOptions = {
  allowedMemberPubkeys?: readonly string[];
};

type PendingWelcomeChannel = {
  channelId: string;
  createdAt: number;
};

const welcomeChannelInput = {
  name: WELCOME_CHANNEL_NAME,
  channelType: "stream",
  visibility: "private",
  description: WELCOME_CHANNEL_DESCRIPTION,
} satisfies CreateChannelInput;

function hasOnlyCurrentOrAllowedMembers(
  channel: Channel,
  allowedMemberPubkeys: readonly string[] = [],
) {
  const allowed = new Set(
    allowedMemberPubkeys.map((pubkey) => pubkey.toLowerCase()),
  );
  if (channel.memberPubkeys.length > 0) {
    const nonAllowedMembers = channel.memberPubkeys.filter(
      (pubkey) => !allowed.has(pubkey.toLowerCase()),
    );
    return nonAllowedMembers.length <= 1;
  }

  if (channel.memberCount > allowed.size + 1) {
    return false;
  }

  return true;
}

export function isWelcomeChannel(channel: Channel | null | undefined) {
  return (
    channel !== null &&
    channel !== undefined &&
    channel.name === WELCOME_CHANNEL_NAME &&
    channel.channelType === "stream" &&
    channel.visibility === "private"
  );
}

function isPrivateWelcomeChannelCandidate(channel: Channel) {
  return (
    isWelcomeChannel(channel) && channel.archivedAt === null && channel.isMember
  );
}

export function isPrivateWelcomeChannel(
  channel: Channel,
  options: WelcomeChannelOptions = {},
) {
  return (
    isPrivateWelcomeChannelCandidate(channel) &&
    hasOnlyCurrentOrAllowedMembers(channel, options.allowedMemberPubkeys)
  );
}

export function findPrivateWelcomeChannel(
  channels: Channel[],
  options: WelcomeChannelOptions = {},
) {
  return (
    channels.find((channel) => isPrivateWelcomeChannel(channel, options)) ??
    null
  );
}

async function hasOnlyCurrentHumanMember(
  client: WelcomeChannelClient,
  channel: Channel,
) {
  if (!client.getChannelMembers) {
    return false;
  }

  try {
    const members = await client.getChannelMembers(channel.id);
    return members.filter((member) => !member.isAgent).length <= 1;
  } catch {
    return false;
  }
}

async function ensureCurrentWelcomeChannelMetadata(
  client: WelcomeChannelClient,
  channel: Channel,
) {
  const input: UpdateChannelInput = {
    channelId: channel.id,
  };

  if (channel.description !== WELCOME_CHANNEL_DESCRIPTION) {
    input.description = WELCOME_CHANNEL_DESCRIPTION;
  }

  if (channel.ttlSeconds !== null || channel.ttlDeadline !== null) {
    input.ttlSeconds = null;
  }

  if (input.description === undefined && input.ttlSeconds === undefined) {
    return channel;
  }

  if (!client.updateChannel) {
    return channel;
  }

  return client.updateChannel(input);
}

export async function ensureWelcomeChannel(
  client: WelcomeChannelClient,
  options: WelcomeChannelOptions = {},
) {
  const channels = await client.getChannels();
  const existingWelcome = findPrivateWelcomeChannel(channels, options);
  if (existingWelcome) {
    return ensureCurrentWelcomeChannelMetadata(client, existingWelcome);
  }

  for (const channel of channels.filter(isPrivateWelcomeChannelCandidate)) {
    if (await hasOnlyCurrentHumanMember(client, channel)) {
      return ensureCurrentWelcomeChannelMetadata(client, channel);
    }
  }

  return client.createChannel(welcomeChannelInput);
}

export function welcomeChannelEnsuredStorageKey(
  pubkey: string,
  workspaceScope: string,
) {
  return `${WELCOME_CHANNEL_ENSURED_STORAGE_KEY}:${encodeURIComponent(
    workspaceScope,
  )}:${pubkey}`;
}

export function hasEnsuredWelcomeChannel(
  pubkey: string | null | undefined,
  workspaceScope: string | null | undefined,
) {
  if (typeof window === "undefined" || !pubkey || !workspaceScope) {
    return false;
  }

  try {
    return (
      window.localStorage.getItem(
        welcomeChannelEnsuredStorageKey(pubkey, workspaceScope),
      ) === "true"
    );
  } catch {
    return false;
  }
}

export function markWelcomeChannelEnsured(
  pubkey: string | null | undefined,
  workspaceScope: string | null | undefined,
) {
  if (typeof window === "undefined" || !pubkey || !workspaceScope) {
    return;
  }

  try {
    window.localStorage.setItem(
      welcomeChannelEnsuredStorageKey(pubkey, workspaceScope),
      "true",
    );
  } catch {
    // Best-effort. The channel itself is the important durable state.
  }
}

function readPendingWelcomeChannel(): PendingWelcomeChannel | null {
  return readPendingWelcomeChannelFromStorage(
    PENDING_WELCOME_CHANNEL_STORAGE_KEY,
  );
}

function readPendingWelcomeChannelFromStorage(
  storageKey: string,
): PendingWelcomeChannel | null {
  if (typeof window === "undefined") {
    return null;
  }

  try {
    const raw = window.sessionStorage.getItem(storageKey);
    if (!raw) {
      return null;
    }

    const parsed = JSON.parse(raw) as Partial<PendingWelcomeChannel>;
    if (
      typeof parsed.channelId !== "string" ||
      parsed.channelId.length === 0 ||
      typeof parsed.createdAt !== "number"
    ) {
      return null;
    }

    return {
      channelId: parsed.channelId,
      createdAt: parsed.createdAt,
    };
  } catch {
    return null;
  }
}

function clearPendingWelcomeChannel() {
  clearPendingWelcomeChannelFromStorage(PENDING_WELCOME_CHANNEL_STORAGE_KEY);
}

function clearPendingWelcomeChannelFromStorage(storageKey: string) {
  if (typeof window === "undefined") {
    return;
  }

  try {
    window.sessionStorage.removeItem(storageKey);
  } catch {
    // Best-effort. A stale pending channel expires automatically.
  }
}

export function rememberPendingWelcomeChannel(channelId: string) {
  if (typeof window === "undefined") {
    return;
  }

  try {
    window.sessionStorage.setItem(
      PENDING_WELCOME_CHANNEL_STORAGE_KEY,
      JSON.stringify({ channelId, createdAt: Date.now() }),
    );
    window.sessionStorage.setItem(
      WELCOME_INITIAL_UNREAD_SUPPRESSION_STORAGE_KEY,
      JSON.stringify({ channelId, createdAt: Date.now() }),
    );
  } catch {
    // Best-effort. The user can still select the channel from the sidebar.
  }
}

export function hasPendingWelcomeInitialUnreadSuppression(channelId: string) {
  const pending = readPendingWelcomeChannelFromStorage(
    WELCOME_INITIAL_UNREAD_SUPPRESSION_STORAGE_KEY,
  );
  if (!pending) {
    return false;
  }

  if (Date.now() - pending.createdAt > PENDING_WELCOME_CHANNEL_MAX_AGE_MS) {
    return false;
  }

  return pending.channelId === channelId;
}

export function consumePendingWelcomeInitialUnreadSuppression(
  channelId: string,
) {
  if (!hasPendingWelcomeInitialUnreadSuppression(channelId)) {
    return false;
  }

  clearPendingWelcomeChannelFromStorage(
    WELCOME_INITIAL_UNREAD_SUPPRESSION_STORAGE_KEY,
  );
  return true;
}

export function notifyWelcomeChannelReady(channelId: string) {
  if (typeof window === "undefined") {
    return;
  }

  window.dispatchEvent(
    new CustomEvent(WELCOME_CHANNEL_READY_EVENT, {
      detail: { channelId },
    }),
  );
}

export function consumePendingWelcomeChannel(
  availableChannelIds: ReadonlySet<string>,
) {
  const pending = readPendingWelcomeChannel();
  if (!pending) {
    return null;
  }

  if (Date.now() - pending.createdAt > PENDING_WELCOME_CHANNEL_MAX_AGE_MS) {
    clearPendingWelcomeChannel();
    return null;
  }

  if (!availableChannelIds.has(pending.channelId)) {
    return null;
  }

  clearPendingWelcomeChannel();
  return pending.channelId;
}
