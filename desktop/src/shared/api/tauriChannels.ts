import type {
  Channel,
  ChannelDetail,
  ChannelMember,
  ChannelMessagesPageResponse,
  ChannelPageCursor,
  ChannelType,
  CreateChannelInput,
  SetChannelPurposeInput,
  SetChannelTopicInput,
  UpdateChannelInput,
} from "@/shared/api/types";
import { invokeTauri } from "@/shared/api/tauri";

export type RawChannel = {
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

/**
 * Response payload for the `get_channels` Tauri command.
 *
 * When `channels` is `null`, the caller's `knownHash` matched the relay
 * snapshot and the expensive channel list was not re-serialized across IPC.
 * `lastMessages` is always present so the caller can update sidebar
 * timestamps without a full channel-list re-render.
 */
export type GetChannelsPayload = {
  hash: string;
  /** Full channel list, or `null` on a not-modified (hash-match) response. */
  channels: Channel[] | null;
  /** Map of channel id → ISO-8601 timestamp of its most recent message. */
  lastMessages: Record<string, string>;
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

export function fromRawChannel(channel: RawChannel): Channel {
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

export function fromRawChannelDetail(channel: RawChannelDetail): ChannelDetail {
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

/**
 * Fetch the channel list from the backend.
 *
 * Pass `knownHash` from a previous response to enable the not-modified
 * short-circuit: when the relay snapshot is unchanged, `channels` in the
 * returned payload will be `null` so the multi-MB list is not deserialized.
 * Pass `null` to always request the full list.
 */
export async function getChannels(
  knownHash: string | null,
): Promise<GetChannelsPayload> {
  const raw = await invokeTauri<{
    hash: string;
    channels: RawChannel[] | null;
    last_messages: Record<string, string>;
  }>("get_channels", { knownHash });
  return {
    hash: raw.hash,
    channels: raw.channels !== null ? raw.channels.map(fromRawChannel) : null,
    lastMessages: raw.last_messages,
  };
}

/**
 * Fetch the open-channel directory: every joinable open channel plus the
 * active identity's own channels, each carrying its `isMember` flag.
 *
 * `getChannels` intentionally omits this discovery superset from the 60s poll
 * because it requires an unbounded all-open relay scan. Call this on demand —
 * when the channel browser opens or a global search goes active — with a
 * generous staleTime so the expensive scan runs only when a user is actually
 * looking for channels to join.
 */
export async function getOpenChannelDirectory(): Promise<Channel[]> {
  return (await invokeTauri<RawChannel[]>("get_open_channel_directory")).map(
    fromRawChannel,
  );
}

export async function createChannel(
  input: CreateChannelInput,
): Promise<Channel> {
  return fromRawChannel(await invokeTauri<RawChannel>("create_channel", input));
}

export async function ensureStarterChannels(): Promise<Channel[]> {
  return (await invokeTauri<RawChannel[]>("ensure_starter_channels")).map(
    fromRawChannel,
  );
}

export type OpenDmInput = {
  pubkeys: string[];
  /**
   * Tenant scope captured by the caller before its first await (community
   * relay URL). The backend fails closed when the active community no longer
   * matches, so a suspended callback can never open a DM in the wrong
   * community. Omit for callers without a tenant boundary.
   */
  expectedRelayUrl?: string;
  /**
   * Signer identity captured together with the relay scope (owner pubkey,
   * hex). Relay and keys change under separate locks during a community
   * switch, so the backend also fails closed when the active identity no
   * longer matches — a stale callback can neither open the DM in the wrong
   * community nor open it under the wrong identity.
   */
  expectedSignerPubkey?: string;
};

export async function openDm(input: OpenDmInput): Promise<Channel> {
  return fromRawChannel(await invokeTauri<RawChannel>("open_dm", input));
}

export async function hideDm(channelId: string): Promise<void> {
  await invokeTauri<void>("hide_dm", { channelId });
}

export async function getChannelDetails(
  channelId: string,
): Promise<ChannelDetail> {
  const detail = await invokeTauri<RawChannelDetail>("get_channel_details", {
    channelId,
  });
  return fromRawChannelDetail(detail);
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

type RawChannelMessagesPageResponse = {
  events: ChannelMessagesPageResponse["events"];
  next_cursor: { created_at: number; event_id: string } | null;
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

export async function getChannelMembers(
  channelId: string,
): Promise<ChannelMember[]> {
  const response = await invokeTauri<RawChannelMembersResponse>(
    "get_channel_members",
    { channelId },
  );
  return response.members.map(fromRawChannelMember);
}

export async function joinChannel(channelId: string): Promise<void> {
  await invokeTauri<void>("join_channel", { channelId });
}
