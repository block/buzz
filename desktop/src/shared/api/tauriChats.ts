import { invokeTauri } from "@/shared/api/tauri";
import type {
  Channel,
  ChannelType,
  ChatMetadata,
  CreateChatInput,
  SendChannelMessageResult,
  SendChatContextMessageInput,
  UpdateChatMetadataInput,
} from "@/shared/api/types";

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
  created_at?: string | null;
  last_message_at: string | null;
  archived_at: string | null;
  participants: string[];
  participant_pubkeys: string[];
  is_member?: boolean;
  ttl_seconds: number | null;
  ttl_deadline: string | null;
};

type RawChatMetadata = {
  channel_id: string;
  author_pubkey?: string | null;
  title: string | null;
  default_agent_pubkey: string | null;
  template_id: string | null;
  project_id: string | null;
  project_name: string | null;
  project_path: string | null;
  project_template_id: string | null;
  source_channel_id: string | null;
  source_event_id: string | null;
  source_thread_root_id: string | null;
  updated_at: number;
};

type RawSendChannelMessageResult = {
  event_id: string;
  parent_event_id: string | null;
  root_event_id: string | null;
  depth: number;
  created_at: number;
};

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
    createdAt: channel.created_at ?? null,
    lastMessageAt: channel.last_message_at,
    archivedAt: channel.archived_at,
    participants: channel.participants,
    participantPubkeys: channel.participant_pubkeys,
    isMember: channel.is_member ?? true,
    ttlSeconds: channel.ttl_seconds,
    ttlDeadline: channel.ttl_deadline,
  };
}

function fromRawChatMetadata(metadata: RawChatMetadata): ChatMetadata {
  return {
    channelId: metadata.channel_id,
    authorPubkey: metadata.author_pubkey ?? null,
    title: metadata.title,
    defaultAgentPubkey: metadata.default_agent_pubkey,
    templateId: metadata.template_id,
    projectId: metadata.project_id,
    projectName: metadata.project_name,
    projectPath: metadata.project_path,
    projectTemplateId: metadata.project_template_id,
    sourceChannelId: metadata.source_channel_id,
    sourceEventId: metadata.source_event_id,
    sourceThreadRootId: metadata.source_thread_root_id,
    updatedAt: metadata.updated_at,
  };
}

export async function listChats(): Promise<Channel[]> {
  const chats = await invokeTauri<RawChannel[]>("list_chats");
  return chats.map(fromRawChannel);
}

export async function listChatMetadata(): Promise<ChatMetadata[]> {
  const metadata = await invokeTauri<RawChatMetadata[]>("list_chat_metadata");
  return metadata.map(fromRawChatMetadata);
}

export async function createChat(input: CreateChatInput): Promise<Channel> {
  return fromRawChannel(
    await invokeTauri<RawChannel>("create_chat", { input }),
  );
}

export async function getChatMetadata(
  channelId: string,
): Promise<ChatMetadata | null> {
  const metadata = await invokeTauri<RawChatMetadata | null>(
    "get_chat_metadata",
    { channelId },
  );
  return metadata ? fromRawChatMetadata(metadata) : null;
}

export async function updateChatMetadata(
  input: UpdateChatMetadataInput,
): Promise<ChatMetadata> {
  const metadata = await invokeTauri<RawChatMetadata>("update_chat_metadata", {
    input,
  });
  return fromRawChatMetadata(metadata);
}

export async function sendChatContextMessage(
  input: SendChatContextMessageInput,
): Promise<SendChannelMessageResult> {
  const result = await invokeTauri<RawSendChannelMessageResult>(
    "send_chat_context_message",
    {
      input,
    },
  );
  return {
    eventId: result.event_id,
    parentEventId: result.parent_event_id,
    rootEventId: result.root_event_id,
    depth: result.depth,
    createdAt: result.created_at,
  };
}

export async function pickChatProjectDirectory(): Promise<string | null> {
  return invokeTauri<string | null>("pick_team_directory");
}
