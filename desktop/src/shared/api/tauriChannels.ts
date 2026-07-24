import type {
  Channel,
  ChannelDetail,
  ChannelMember,
  ChannelMessagesPageResponse,
  ChannelPageCursor,
  ChannelType,
  CreateChannelInput,
  OpenDmInput,
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

export async function getChannels(): Promise<Channel[]> {
  const channels = await invokeTauri<RawChannel[]>("get_channels");
  return channels.map(fromRawChannel);
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
