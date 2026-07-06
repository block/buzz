import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type {
  AddChannelMembersInput,
  AddChannelMembersResult,
  BackendProviderCandidate,
  BackendProviderProbeResult,
  CanvasResponse,
  Channel,
  ChannelDetail,
  ChannelMember,
  ChannelMessagesPageResponse,
  ChannelPageCursor,
  ChannelType,
  CreateChannelInput,
  GetHomeFeedInput,
  HomeFeedResponse,
  Identity,
  ManagedAgent,
  ManagedAgentBackend,
  RelayAgent,
  RelayMember,
  RelayMemberRole,
  PresenceLookup,
  PresenceStatus,
  Profile,
  RelayEvent,
  SearchMessagesInput,
  SearchMessagesResponse,
  SendChannelMessageResult,
  SetCanvasInput,
  SetCanvasResult,
  SetChannelPurposeInput,
  SetChannelTopicInput,
  ThreadCursor,
  ThreadRepliesResponse,
  UpdateProfileInput,
  UpdateChannelInput,
  UserProfileSummary,
  UserSearchPage,
  UserSearchResult,
  UsersBatchResponse,
  CreateManagedAgentInput,
  AgentModelsResponse,
  UpdateManagedAgentInput,
  AcpAvailabilityStatus,
  AcpRuntimeCatalogEntry,
  CommandAvailability,
  InstallRuntimeResult,
  OpenDmInput,
  RuntimeConfigSurface,
} from "@/shared/api/types";

type RawIdentity = { pubkey: string; display_name: string };

type RawProfile = {
  pubkey: string;
  display_name: string | null;
  avatar_url: string | null;
  about: string | null;
  nip05_handle: string | null;
  owner_pubkey: string | null;
};

type RawUserProfileSummary = Omit<RawProfile, "pubkey" | "about"> & {
  is_agent?: boolean;
};

type RawUsersBatchResponse = {
  profiles: Record<string, RawUserProfileSummary>;
  missing: string[];
};

type RawUserSearchResult = RawUserProfileSummary & { pubkey: string };

type RawSearchUsersResponse = {
  users: RawUserSearchResult[];
  next_cursor?: string | null;
};

type RawPresenceLookup = Record<string, PresenceStatus>;

type RawChannel = {
  id: string;
  name: string;
  channel_type: ChannelType;
  visibility: "open" | "private";
  description: string;
  topic: string | null;
  purpose: string | null;
  member_count: number;
  member_pubkeys: string[];
  last_message_at: string | null;
  archived_at: string | null;
  participants: string[];
  participant_pubkeys: string[];
  is_member?: boolean;
  ttl_seconds: number | null;
  ttl_deadline: string | null;
};

type RawChannelDetail = RawChannel & {
  created_by: string;
  created_at: string;
  updated_at: string;
  topic_set_by: string | null;
  topic_set_at: string | null;
  purpose_set_by: string | null;
  purpose_set_at: string | null;
  topic_required: boolean;
  max_members: number | null;
  nip29_group_id: string | null;
};

type RawChannelMember = {
  pubkey: string;
  role: ChannelMember["role"];
  is_agent?: boolean;
  joined_at: string;
  display_name: string | null;
};

type RawChannelMembersResponse = {
  members: RawChannelMember[];
  next_cursor: string | null;
};

type RawAddChannelMembersResult = {
  added: string[];
  errors: Array<{
    pubkey: string;
    error: string;
  }>;
};

type RawFeedItem = {
  id: string;
  kind: number;
  pubkey: string;
  content: string;
  created_at: number;
  channel_id: string | null;
  channel_name: string;
  channel_type: string;
  tags: string[][];
  category: "mention" | "needs_action" | "activity" | "agent_activity";
};

type RawHomeFeedResponse = {
  feed: {
    mentions: RawFeedItem[];
    needs_action: RawFeedItem[];
    activity: RawFeedItem[];
    agent_activity: RawFeedItem[];
  };
  meta: {
    since: number;
    total: number;
    generated_at: number;
  };
};

type RawSearchHit = {
  event_id: string;
  content: string;
  kind: number;
  pubkey: string;
  channel_id: string | null;
  channel_name: string | null;
  created_at: number;
  score: number;
};

type RawSearchResponse = {
  hits: RawSearchHit[];
  found: number;
};

type RawSendChannelMessageResult = {
  event_id: string;
  parent_event_id: string | null;
  root_event_id: string | null;
  depth: number;
  created_at: number;
};

type RawRelayAgent = {
  pubkey: string;
  name: string;
  agent_type: string;
  channels: string[];
  channel_ids: string[];
  capabilities: string[];
  status: RelayAgent["status"];
  respond_to?: RelayAgent["respondTo"];
  respond_to_allowlist?: string[];
};
export type RawManagedAgent = {
  pubkey: string;
  name: string;
  persona_id: string | null;
  relay_url: string;
  acp_command: string;
  agent_command: string;
  agent_command_override?: string | null;
  agent_args: string[];
  mcp_command: string;
  turn_timeout_seconds: number;
  idle_timeout_seconds: number | null;
  max_turn_duration_seconds: number | null;
  parallelism: number;
  system_prompt: string | null;
  avatar_url?: string | null;
  model: string | null;
  provider: string | null;
  persona_out_of_date: boolean;
  persona_orphaned: boolean;
  mcp_toolsets: string | null;
  env_vars?: Record<string, string>;
  status: ManagedAgent["status"];
  pid: number | null;
  created_at: string;
  updated_at: string;
  last_started_at: string | null;
  last_stopped_at: string | null;
  last_exit_code: number | null;
  last_error: string | null;
  log_path: string;
  start_on_app_launch: boolean;
  backend: ManagedAgentBackend;
  backend_agent_id: string | null;
  // Optional: pre-feature mock fixtures may omit these. Mapped to
  // `"owner-only"` / `[]` in `fromRawManagedAgent`.
  respond_to?: ManagedAgent["respondTo"];
  respond_to_allowlist?: string[];
};

type RawCreateManagedAgentResponse = {
  agent: RawManagedAgent;
  private_key_nsec: string;
  profile_sync_error: string | null;
  spawn_error: string | null;
};

type RawManagedAgentLog = {
  content: string;
  log_path: string;
};

export type RawAcpRuntimeCatalogEntry = {
  id: string;
  label: string;
  avatar_url: string;
  availability: AcpAvailabilityStatus;
  command: string | null;
  binary_path: string | null;
  default_args: string[];
  mcp_command: string | null;
  install_hint: string;
  install_instructions_url: string;
  can_auto_install: boolean;
  underlying_cli_path: string | null;
};

export type RawInstallStepResult = {
  step: string;
  command: string;
  success: boolean;
  stdout: string;
  stderr: string;
  exit_code: number | null;
};

export type RawInstallRuntimeResult = {
  success: boolean;
  steps: RawInstallStepResult[];
};

type RawCommandAvailability = {
  command: string;
  resolved_path: string | null;
  available: boolean;
};

type RawManagedAgentPrereqs = {
  acp: RawCommandAvailability;
  mcp: RawCommandAvailability;
};

type RawRelayMember = {
  pubkey: string;
  role: string;
  added_by: string | null;
  created_at: string;
};

type RawListRelayMembersResponse = {
  members: RawRelayMember[];
};

type RawCanvasResponse = {
  content: string | null;
  updated_at: number | null;
  author: string | null;
};

type RawSetCanvasResult = {
  ok: boolean;
  event_id: string;
};

function toTauriError(error: unknown): Error {
  if (error instanceof Error) {
    return error;
  }

  if (typeof error === "string") {
    return new Error(error);
  }

  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof error.message === "string"
  ) {
    return new Error(error.message);
  }

  try {
    return new Error(JSON.stringify(error));
  } catch {
    return new Error("Unknown Tauri error");
  }
}

export async function invokeTauri<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args);
  } catch (error) {
    throw toTauriError(error);
  }
}

function fromRawChannel(channel: RawChannel): Channel {
  return {
    id: channel.id,
    name: channel.name,
    channelType: channel.channel_type,
    visibility: channel.visibility,
    description: channel.description,
    topic: channel.topic,
    purpose: channel.purpose,
    memberCount: channel.member_count,
    memberPubkeys: channel.member_pubkeys ?? [],
    lastMessageAt: channel.last_message_at,
    archivedAt: channel.archived_at,
    participants: channel.participants,
    participantPubkeys: channel.participant_pubkeys,
    isMember: channel.is_member ?? true,
    ttlSeconds: channel.ttl_seconds,
    ttlDeadline: channel.ttl_deadline,
  };
}

function fromRawChannelDetail(channel: RawChannelDetail): ChannelDetail {
  return {
    ...fromRawChannel(channel),
    createdBy: channel.created_by,
    createdAt: channel.created_at,
    updatedAt: channel.updated_at,
    topicSetBy: channel.topic_set_by,
    topicSetAt: channel.topic_set_at,
    purposeSetBy: channel.purpose_set_by,
    purposeSetAt: channel.purpose_set_at,
    topicRequired: channel.topic_required,
    maxMembers: channel.max_members,
    nip29GroupId: channel.nip29_group_id,
  };
}

function fromRawChannelMember(member: RawChannelMember): ChannelMember {
  return {
    pubkey: member.pubkey,
    role: member.role,
    isAgent: member.is_agent ?? false,
    joinedAt: member.joined_at,
    displayName: member.display_name,
  };
}

function fromRawFeedItem(item: RawFeedItem) {
  return {
    id: item.id,
    kind: item.kind,
    pubkey: item.pubkey,
    content: item.content,
    createdAt: item.created_at,
    channelId: item.channel_id,
    channelName: item.channel_name,
    channelType: item.channel_type,
    tags: item.tags,
    category: item.category,
  };
}

function fromRawSearchHit(hit: RawSearchHit) {
  return {
    eventId: hit.event_id,
    content: hit.content,
    kind: hit.kind,
    pubkey: hit.pubkey,
    channelId: hit.channel_id,
    channelName: hit.channel_name,
    createdAt: hit.created_at,
    score: hit.score,
  };
}

function fromRawProfile(profile: RawProfile): Profile {
  return {
    pubkey: profile.pubkey,
    displayName: profile.display_name,
    avatarUrl: profile.avatar_url,
    about: profile.about,
    nip05Handle: profile.nip05_handle,
    ownerPubkey: profile.owner_pubkey,
  };
}

function fromRawUserProfileSummary(
  profile: RawUserProfileSummary,
): UserProfileSummary {
  return {
    displayName: profile.display_name,
    avatarUrl: profile.avatar_url,
    nip05Handle: profile.nip05_handle,
    ownerPubkey: profile.owner_pubkey,
    isAgent: profile.is_agent ?? false,
  };
}

function fromRawUserSearchResult(user: RawUserSearchResult): UserSearchResult {
  return {
    pubkey: user.pubkey,
    displayName: user.display_name,
    avatarUrl: user.avatar_url,
    nip05Handle: user.nip05_handle,
    ownerPubkey: user.owner_pubkey,
    isAgent: user.is_agent ?? false,
  };
}

export async function getIdentity(): Promise<Identity> {
  const identity = await invokeTauri<RawIdentity>("get_identity");

  return {
    pubkey: identity.pubkey,
    displayName: identity.display_name,
  };
}

export async function getNsec(): Promise<string> {
  return invokeTauri<string>("get_nsec");
}

export async function importIdentity(nsec: string): Promise<Identity> {
  const raw = await invokeTauri<RawIdentity>("import_identity", { nsec });
  return { pubkey: raw.pubkey, displayName: raw.display_name };
}

export async function getProfile(): Promise<Profile> {
  const profile = await invokeTauri<RawProfile>("get_profile");
  return fromRawProfile(profile);
}

export async function updateProfile(
  input: UpdateProfileInput,
): Promise<Profile> {
  const profile = await invokeTauri<RawProfile>("update_profile", input);
  return fromRawProfile(profile);
}

export async function getUserProfile(pubkey?: string): Promise<Profile> {
  const profile = await invokeTauri<RawProfile>("get_user_profile", { pubkey });
  return fromRawProfile(profile);
}

export async function getUsersBatch(
  pubkeys: string[],
): Promise<UsersBatchResponse> {
  const response = await invokeTauri<RawUsersBatchResponse>("get_users_batch", {
    pubkeys,
  });

  return {
    profiles: Object.fromEntries(
      Object.entries(response.profiles).map(([pubkey, profile]) => [
        pubkey,
        fromRawUserProfileSummary(profile),
      ]),
    ),
    missing: response.missing,
  };
}

export async function searchUsers(
  query: string,
  limit = 8,
  cursor?: string | null,
): Promise<UserSearchPage> {
  const response = await invokeTauri<RawSearchUsersResponse>("search_users", {
    query,
    limit,
    cursor: cursor ?? null,
  });
  return {
    users: response.users.map(fromRawUserSearchResult),
    nextCursor: response.next_cursor ?? null,
  };
}

export async function getPresence(pubkeys: string[]): Promise<PresenceLookup> {
  const response = await invokeTauri<RawPresenceLookup>("get_presence", {
    pubkeys,
  });

  return Object.fromEntries(
    Object.entries(response).map(([pubkey, status]) => [
      pubkey.toLowerCase(),
      status,
    ]),
  );
}

export function getDefaultRelayUrl(): Promise<string> {
  return invokeTauri<string>("get_default_relay_url");
}

export function isSharedIdentity(): Promise<boolean> {
  return invokeTauri<boolean>("is_shared_identity");
}

export function getRelayWsUrl(): Promise<string> {
  return invokeTauri<string>("get_relay_ws_url");
}

export function getRelayHttpUrl(): Promise<string> {
  return invokeTauri<string>("get_relay_http_url");
}

export async function getChannels(): Promise<Channel[]> {
  const channels = await invokeTauri<RawChannel[]>("get_channels");
  return channels.map(fromRawChannel);
}

export async function createChannel(
  input: CreateChannelInput,
): Promise<Channel> {
  return fromRawChannel(await invokeTauri<RawChannel>("create_channel", input));
}

export async function openDm(input: OpenDmInput): Promise<Channel> {
  return fromRawChannel(await invokeTauri<RawChannel>("open_dm", input));
}

export async function hideDm(channelId: string): Promise<void> {
  await invokeTauri<void>("hide_dm", { channelId });
}

export async function getChannelDetails(
  channelId: string,
): Promise<ChannelDetail> {
  const channel = await invokeTauri<RawChannelDetail>("get_channel_details", {
    channelId,
  });
  return fromRawChannelDetail(channel);
}

export async function getChannelMembers(
  channelId: string,
): Promise<ChannelMember[]> {
  const response = await invokeTauri<RawChannelMembersResponse>(
    "get_channel_members",
    {
      channelId,
    },
  );
  return response.members.map(fromRawChannelMember);
}

export async function updateChannel(
  input: UpdateChannelInput,
): Promise<ChannelDetail> {
  const channel = await invokeTauri<RawChannelDetail>("update_channel", {
    input,
  });
  return fromRawChannelDetail(channel);
}

export async function setChannelTopic(
  input: SetChannelTopicInput,
): Promise<void> {
  await invokeTauri("set_channel_topic", input);
}

export async function setChannelPurpose(
  input: SetChannelPurposeInput,
): Promise<void> {
  await invokeTauri("set_channel_purpose", input);
}

export async function archiveChannel(channelId: string): Promise<void> {
  await invokeTauri("archive_channel", { channelId });
}

export async function unarchiveChannel(channelId: string): Promise<void> {
  await invokeTauri("unarchive_channel", { channelId });
}

export async function deleteChannel(channelId: string): Promise<void> {
  await invokeTauri("delete_channel", { channelId });
}

export async function addChannelMembers(
  input: AddChannelMembersInput,
): Promise<AddChannelMembersResult> {
  return invokeTauri<RawAddChannelMembersResult>("add_channel_members", input);
}

export async function removeChannelMember(
  channelId: string,
  pubkey: string,
): Promise<void> {
  await invokeTauri("remove_channel_member", { channelId, pubkey });
}

export async function changeChannelMemberRole(
  channelId: string,
  pubkey: string,
  role: string,
): Promise<void> {
  await invokeTauri("change_channel_member_role", { channelId, pubkey, role });
}

export async function joinChannel(channelId: string): Promise<void> {
  await invokeTauri("join_channel", { channelId });
}

export async function leaveChannel(channelId: string): Promise<void> {
  await invokeTauri("leave_channel", { channelId });
}

export async function getCanvas(channelId: string): Promise<CanvasResponse> {
  const response = await invokeTauri<RawCanvasResponse>("get_canvas", {
    channelId,
  });
  return {
    content: response.content,
    updatedAt: response.updated_at,
    author: response.author,
  };
}

export async function setCanvas(
  input: SetCanvasInput,
): Promise<SetCanvasResult> {
  const response = await invokeTauri<RawSetCanvasResult>("set_canvas", {
    channelId: input.channelId,
    content: input.content,
  });
  return {
    ok: response.ok,
    eventId: response.event_id,
  };
}

export async function getHomeFeed(
  input: GetHomeFeedInput = {},
): Promise<HomeFeedResponse> {
  const response = await invokeTauri<RawHomeFeedResponse>("get_feed", input);

  return {
    feed: {
      mentions: response.feed.mentions.map(fromRawFeedItem),
      needsAction: response.feed.needs_action.map(fromRawFeedItem),
      activity: response.feed.activity.map(fromRawFeedItem),
      agentActivity: response.feed.agent_activity.map(fromRawFeedItem),
    },
    meta: {
      since: response.meta.since,
      total: response.meta.total,
      generatedAt: response.meta.generated_at,
    },
  };
}

export async function searchMessages(
  input: SearchMessagesInput,
): Promise<SearchMessagesResponse> {
  const response = await invokeTauri<RawSearchResponse>("search_messages", {
    q: input.q,
    limit: input.limit,
    channelId: input.channelId,
  });

  return {
    hits: response.hits.map(fromRawSearchHit),
    found: response.found,
  };
}

export async function getEventById(eventId: string): Promise<RelayEvent> {
  const eventJson = await invokeTauri<string>("get_event", { eventId });
  return JSON.parse(eventJson) as RelayEvent;
}

type RawThreadCursor = {
  created_at: number;
  event_id: string;
};

type RawThreadRepliesResponse = {
  events: RelayEvent[];
  next_cursor: RawThreadCursor | null;
};

/**
 * Fetch the full reply subtree under a thread root, server-side.
 *
 * Unlike the channel timeline (which the desktop assembles from its local
 * cache), this walks `thread_metadata` on the relay, so a thread renders
 * complete even when its replies fell outside the channel cold-load window —
 * the descendant gap that made deep/old threads silently incomplete.
 *
 * Paging is forward keyset on `(createdAt, eventId)`: pass the returned
 * `nextCursor` back as `cursor` for the next page. `nextCursor` is non-null only
 * when a full page was returned. The returned `events` are raw nostr events
 * (`RelayEvent`), chronological (oldest first). These are the *replies* under
 * the root (depth >= 1); the root event itself is NOT returned (the relay query
 * keys on `root_event_id`, which a root row lacks). The caller already holds
 * the root — it is the open thread head.
 */
export async function getThreadReplies(
  rootEventId: string,
  channelId?: string | null,
  options?: {
    limit?: number;
    depthLimit?: number;
    cursor?: ThreadCursor | null;
  },
): Promise<ThreadRepliesResponse> {
  const response = await invokeTauri<RawThreadRepliesResponse>(
    "get_thread_replies",
    {
      rootEventId,
      channelId: channelId ?? null,
      limit: options?.limit ?? null,
      depthLimit: options?.depthLimit ?? null,
      cursor: options?.cursor
        ? {
            created_at: options.cursor.createdAt,
            event_id: options.cursor.eventId,
          }
        : null,
    },
  );

  return {
    events: response.events,
    nextCursor: response.next_cursor
      ? {
          createdAt: response.next_cursor.created_at,
          eventId: response.next_cursor.event_id,
        }
      : null,
  };
}

type RawChannelMessagesPageResponse = {
  events: RelayEvent[];
  next_cursor: RawThreadCursor | null;
};

/**
 * Fetch one keyset page of top-level channel history strictly older than a
 * cursor, via the bridge composite `(createdAt, eventId)` cursor.
 *
 * The desktop timeline pages history over WS `REQ` with a bare `until`
 * (`createdAt`) cursor, which cannot advance past a `createdAt` second denser
 * than one page. This is the escape hatch: `beforeId` is the id of the oldest
 * event already loaded at `before`, and the relay returns strictly-older rows
 * (`created_at < before OR (created_at = before AND id > beforeId)`). Pass the
 * returned `nextCursor` back to page further; `nextCursor` is null once a short
 * page proves history is exhausted.
 */
export async function getChannelMessagesBefore(
  channelId: string,
  cursor: ChannelPageCursor,
  limit?: number,
): Promise<ChannelMessagesPageResponse> {
  const response = await invokeTauri<RawChannelMessagesPageResponse>(
    "get_channel_messages_before",
    {
      channelId,
      before: cursor.createdAt,
      beforeId: cursor.eventId,
      limit: limit ?? null,
    },
  );

  return {
    events: response.events,
    nextCursor: response.next_cursor
      ? {
          createdAt: response.next_cursor.created_at,
          eventId: response.next_cursor.event_id,
        }
      : null,
  };
}

export async function sendChannelMessage(
  channelId: string,
  content: string,
  parentEventId?: string | null,
  mediaTags?: string[][],
  mentionPubkeys?: string[],
  kind?: number,
  emojiTags?: string[][],
  mentionTags?: string[][],
): Promise<SendChannelMessageResult> {
  const response = await invokeTauri<RawSendChannelMessageResult>(
    "send_channel_message",
    {
      channelId,
      content,
      parentEventId,
      mediaTags: mediaTags ?? null,
      emojiTags: emojiTags ?? null,
      mentionTags: mentionTags ?? null,
      mentionPubkeys: mentionPubkeys ?? null,
      kind: kind ?? null,
    },
  );

  return {
    eventId: response.event_id,
    parentEventId: response.parent_event_id,
    rootEventId: response.root_event_id,
    depth: response.depth,
    createdAt: response.created_at,
  };
}

export type BlobDescriptor = {
  url: string;
  sha256: string;
  size: number;
  type: string;
  uploaded: number;
  dim?: string;
  blurhash?: string;
  thumb?: string;
  duration?: number;
  image?: string;
  /** Original filename captured client-side. */
  filename?: string;
};

export async function uploadMedia(
  filePath: string,
  isTemp: boolean,
): Promise<BlobDescriptor> {
  return invokeTauri<BlobDescriptor>("upload_media", {
    filePath,
    isTemp,
  });
}

export async function pickAndUploadMedia(): Promise<BlobDescriptor[]> {
  return invokeTauri<BlobDescriptor[]>("pick_and_upload_media", {});
}

export async function uploadMediaBytes(
  data: number[],
  filename?: string,
  /** Correlation id for `media-upload-progress` events from the Rust side. */
  progressId?: string,
): Promise<BlobDescriptor> {
  return invokeTauri<BlobDescriptor>("upload_media_bytes", {
    data,
    filename,
    progressId,
  });
}

export async function editMessage(
  channelId: string,
  eventId: string,
  content: string,
  mediaTags?: string[][],
  emojiTags?: string[][],
): Promise<void> {
  await invokeTauri("edit_message", {
    channelId,
    eventId,
    content,
    mediaTags: mediaTags ?? [],
    emojiTags: emojiTags ?? [],
  });
}

export async function deleteMessage(
  channelId: string,
  eventId: string,
): Promise<void> {
  await invokeTauri("delete_message", { channelId, eventId });
}

export async function addReaction(
  eventId: string,
  emoji: string,
  emojiUrl?: string,
): Promise<void> {
  await invokeTauri("add_reaction", { eventId, emoji, emojiUrl });
}

export async function removeReaction(
  eventId: string,
  emoji: string,
): Promise<void> {
  await invokeTauri("remove_reaction", { eventId, emoji });
}

export async function signRelayEvent(input: {
  kind: number;
  content: string;
  createdAt?: number;
  tags: string[][];
}): Promise<RelayEvent> {
  const eventJson = await invokeTauri<string>("sign_event", input);
  return JSON.parse(eventJson) as RelayEvent;
}

export async function createAuthEvent(input: {
  challenge: string;
  relayUrl: string;
}): Promise<RelayEvent> {
  const eventJson = await invokeTauri<string>("create_auth_event", input);
  return JSON.parse(eventJson) as RelayEvent;
}

function fromRawRelayAgent(agent: RawRelayAgent): RelayAgent {
  return {
    pubkey: agent.pubkey,
    name: agent.name,
    agentType: agent.agent_type,
    channels: agent.channels,
    channelIds: agent.channel_ids ?? [],
    capabilities: agent.capabilities,
    status: agent.status,
    respondTo: agent.respond_to ?? null,
    respondToAllowlist: agent.respond_to_allowlist ?? [],
  };
}

export function fromRawManagedAgent(agent: RawManagedAgent): ManagedAgent {
  return {
    pubkey: agent.pubkey,
    name: agent.name,
    personaId: agent.persona_id,
    relayUrl: agent.relay_url,
    acpCommand: agent.acp_command,
    agentCommand: agent.agent_command,
    agentCommandOverride: agent.agent_command_override ?? null,
    agentArgs: agent.agent_args,
    mcpCommand: agent.mcp_command,
    turnTimeoutSeconds: agent.turn_timeout_seconds,
    idleTimeoutSeconds: agent.idle_timeout_seconds,
    maxTurnDurationSeconds: agent.max_turn_duration_seconds,
    parallelism: agent.parallelism,
    systemPrompt: agent.system_prompt,
    avatarUrl: agent.avatar_url ?? null,
    model: agent.model,
    // Fallbacks for pre-feature mocks/fixtures. Real records always carry them.
    provider: agent.provider ?? null,
    personaOutOfDate: agent.persona_out_of_date ?? false,
    personaOrphaned: agent.persona_orphaned ?? false,
    mcpToolsets: agent.mcp_toolsets,
    envVars: agent.env_vars ?? {},
    status: agent.status,
    pid: agent.pid,
    createdAt: agent.created_at,
    updatedAt: agent.updated_at,
    lastStartedAt: agent.last_started_at,
    lastStoppedAt: agent.last_stopped_at,
    lastExitCode: agent.last_exit_code,
    lastError: agent.last_error,
    logPath: agent.log_path,
    startOnAppLaunch: agent.start_on_app_launch,
    backend: agent.backend,
    backendAgentId: agent.backend_agent_id,
    // Fallbacks for pre-feature mocks/fixtures that don't carry these fields.
    // Real agent records always include them (defaulted server-side).
    respondTo: agent.respond_to ?? "owner-only",
    respondToAllowlist: agent.respond_to_allowlist ?? [],
  };
}

function fromRawAcpRuntimeCatalogEntry(
  entry: RawAcpRuntimeCatalogEntry,
): AcpRuntimeCatalogEntry {
  return {
    id: entry.id,
    label: entry.label,
    avatarUrl: entry.avatar_url,
    availability: entry.availability,
    command: entry.command,
    binaryPath: entry.binary_path,
    defaultArgs: entry.default_args,
    mcpCommand: entry.mcp_command,
    installHint: entry.install_hint,
    installInstructionsUrl: entry.install_instructions_url,
    canAutoInstall: entry.can_auto_install,
    underlyingCliPath: entry.underlying_cli_path,
  };
}

function fromRawInstallRuntimeResult(
  raw: RawInstallRuntimeResult,
): InstallRuntimeResult {
  return {
    success: raw.success,
    steps: raw.steps.map((step) => ({
      step: step.step,
      command: step.command,
      success: step.success,
      stdout: step.stdout,
      stderr: step.stderr,
      exitCode: step.exit_code,
    })),
  };
}

function fromRawCommandAvailability(
  command: RawCommandAvailability,
): CommandAvailability {
  return {
    command: command.command,
    resolvedPath: command.resolved_path,
    available: command.available,
  };
}

// ── Relay Members ────────────────────────────────────────────────────────────

function fromRawRelayMember(raw: RawRelayMember): RelayMember {
  return {
    pubkey: raw.pubkey,
    role: raw.role as RelayMemberRole,
    addedBy: raw.added_by,
    createdAt: raw.created_at,
  };
}

export async function listRelayMembers(): Promise<RelayMember[]> {
  const response =
    await invokeTauri<RawListRelayMembersResponse>("list_relay_members");
  return response.members.map(fromRawRelayMember);
}

export async function getMyRelayMembership(): Promise<RelayMember | null> {
  try {
    const raw = await invokeTauri<RawRelayMember>("get_my_relay_membership");
    return fromRawRelayMember(raw);
  } catch (error) {
    // "relay returned 404 Not Found" = not a relay member — return null so
    // the UI hides the Members tab. Re-throw real errors (network, auth, 500)
    // so React Query surfaces them.
    if (
      error instanceof Error &&
      error.message.startsWith("relay returned 404")
    ) {
      return null;
    }
    throw error;
  }
}

export async function addRelayMember(
  targetPubkey: string,
  role: string,
): Promise<void> {
  await invokeTauri("add_relay_member", { targetPubkey, role });
}

export async function removeRelayMember(targetPubkey: string): Promise<void> {
  await invokeTauri("remove_relay_member", { targetPubkey });
}

export async function changeRelayMemberRole(
  targetPubkey: string,
  newRole: string,
): Promise<void> {
  await invokeTauri("change_relay_member_role", { targetPubkey, newRole });
}

export async function listRelayAgents(): Promise<RelayAgent[]> {
  return (await invokeTauri<RawRelayAgent[]>("list_relay_agents")).map(
    fromRawRelayAgent,
  );
}

export async function listManagedAgents(): Promise<ManagedAgent[]> {
  return (await invokeTauri<RawManagedAgent[]>("list_managed_agents")).map(
    fromRawManagedAgent,
  );
}

export async function createManagedAgent(input: CreateManagedAgentInput) {
  const response = await invokeTauri<RawCreateManagedAgentResponse>(
    "create_managed_agent",
    {
      input: {
        name: input.name,
        personaId: input.personaId,
        relayUrl: input.relayUrl,
        acpCommand: input.acpCommand,
        agentCommand: input.agentCommand,
        harnessOverride: input.harnessOverride ?? false,
        agentArgs: input.agentArgs,
        mcpCommand: input.mcpCommand,
        mcpToolsets: input.mcpToolsets,
        turnTimeoutSeconds: input.turnTimeoutSeconds,
        idleTimeoutSeconds: input.idleTimeoutSeconds,
        maxTurnDurationSeconds: input.maxTurnDurationSeconds,
        parallelism: input.parallelism,
        systemPrompt: input.systemPrompt,
        avatarUrl: input.avatarUrl,
        model: input.model,
        envVars: input.envVars ?? {},
        spawnAfterCreate: input.spawnAfterCreate,
        startOnAppLaunch: input.startOnAppLaunch,
        backend: input.backend,
        respondTo: input.respondTo,
        respondToAllowlist: input.respondToAllowlist,
        relayMesh: input.relayMesh,
      },
    },
  );
  return {
    agent: fromRawManagedAgent(response.agent),
    privateKeyNsec: response.private_key_nsec,
    profileSyncError: response.profile_sync_error,
    spawnError: response.spawn_error,
  };
}

export async function startManagedAgent(pubkey: string): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>("start_managed_agent", {
    pubkey,
  });
  return fromRawManagedAgent(response);
}

export async function stopManagedAgent(pubkey: string): Promise<ManagedAgent> {
  const response = await invokeTauri<RawManagedAgent>("stop_managed_agent", {
    pubkey,
  });
  return fromRawManagedAgent(response);
}

export async function deleteManagedAgent(
  pubkey: string,
  forceRemoteDelete?: boolean,
): Promise<void> {
  await invokeTauri("delete_managed_agent", {
    pubkey,
    forceRemoteDelete: forceRemoteDelete ?? null,
  });
}

export async function getManagedAgentLog(pubkey: string, lineCount?: number) {
  const response = await invokeTauri<RawManagedAgentLog>(
    "get_managed_agent_log",
    {
      pubkey,
      lineCount,
    },
  );

  return {
    content: response.content,
    logPath: response.log_path,
  };
}

export async function discoverAcpRuntimes(): Promise<AcpRuntimeCatalogEntry[]> {
  return (
    await invokeTauri<RawAcpRuntimeCatalogEntry[]>("discover_acp_providers")
  ).map(fromRawAcpRuntimeCatalogEntry);
}

export async function installAcpRuntime(
  runtimeId: string,
): Promise<InstallRuntimeResult> {
  const raw = await invokeTauri<RawInstallRuntimeResult>(
    "install_acp_runtime",
    { runtimeId },
  );
  return fromRawInstallRuntimeResult(raw);
}

export async function discoverManagedAgentPrereqs(input: {
  acpCommand?: string;
  mcpCommand?: string;
}) {
  const response = await invokeTauri<RawManagedAgentPrereqs>(
    "discover_managed_agent_prereqs",
    {
      input: {
        acpCommand: input.acpCommand,
        mcpCommand: input.mcpCommand,
      },
    },
  );

  return {
    acp: fromRawCommandAvailability(response.acp),
    mcp: fromRawCommandAvailability(response.mcp),
  };
}

// ── Model discovery ───────────────────────────────────────────────────────────

export async function getAgentModels(pubkey: string) {
  return invokeTauri<AgentModelsResponse>("get_agent_models", { pubkey });
}

export async function getAgentConfigSurface(
  pubkey: string,
): Promise<RuntimeConfigSurface> {
  return invokeTauri<RuntimeConfigSurface>("get_agent_config_surface", {
    pubkey,
  });
}

export async function putAgentSessionConfig(
  pubkey: string,
  payload: unknown,
): Promise<void> {
  return invokeTauri<void>("put_agent_session_config", { pubkey, payload });
}

type RawUpdateManagedAgentResponse = {
  agent: RawManagedAgent;
  profile_sync_error: string | null;
};

export async function updateManagedAgent(
  input: UpdateManagedAgentInput,
): Promise<{ agent: ManagedAgent; profileSyncError: string | null }> {
  const response = await invokeTauri<RawUpdateManagedAgentResponse>(
    "update_managed_agent",
    { input },
  );
  return {
    agent: fromRawManagedAgent(response.agent),
    profileSyncError: response.profile_sync_error,
  };
}

// ── Backend provider discovery ────────────────────────────────────────────────

export async function discoverBackendProviders(): Promise<
  BackendProviderCandidate[]
> {
  return invokeTauri<BackendProviderCandidate[]>("discover_backend_providers");
}

export async function probeBackendProvider(
  binaryPath: string,
): Promise<BackendProviderProbeResult> {
  return invokeTauri<BackendProviderProbeResult>("probe_backend_provider", {
    binaryPath,
  });
}

// ── NIP-44 encrypt-to-self ───────────────────────────────────────────────────

export async function nip44EncryptToSelf(plaintext: string): Promise<string> {
  return invokeTauri<string>("nip44_encrypt_to_self", { plaintext });
}

export async function nip44DecryptFromSelf(
  ciphertext: string,
): Promise<string> {
  return invokeTauri<string>("nip44_decrypt_from_self", { ciphertext });
}

// ── NIP-AB device pairing ───────────────────────────────────────────────────

export async function startPairing(): Promise<string> {
  return invokeTauri<string>("start_pairing");
}

export async function confirmPairingSas(): Promise<void> {
  await invokeTauri("confirm_pairing_sas");
}

export async function cancelPairing(): Promise<void> {
  await invokeTauri("cancel_pairing");
}

export async function applyWorkspace(
  relayUrl: string,
  nsec?: string,
  token?: string,
  reposDir?: string,
): Promise<void> {
  await invokeTauri("apply_workspace", {
    relayUrl,
    nsec: nsec ?? null,
    token: token ?? null,
    reposDir: reposDir ?? null,
  });
}

// Validate a candidate repos dir without mutating the filesystem. Rejects
// with a human-readable reason; resolves for a valid or empty path.
export async function validateReposDir(dir: string): Promise<void> {
  await invokeTauri("validate_repos_dir", { dir });
}

export const setPreventSleepActive = (active: boolean) =>
  invokeTauri("set_prevent_sleep_active", { active });

/** Returns true on macOS, Windows, and Linux AppImage installs.
 *  Returns false on Linux non-AppImage packages (e.g. .deb) where
 *  Tauri's updater cannot swap the binary. */
export function isAutoUpdateSupported(): Promise<boolean> {
  return invokeTauri<boolean>("is_auto_update_supported");
}
